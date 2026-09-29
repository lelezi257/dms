# RFC-0003：写入可见性、持久化与版本提交

状态：Accepted
目标 Milestone：M2
研究依据：[专题二](../architecture/02-write-durability-publication.md)
数据模型：[RFC-0002](0002-file-version-chunk-model.md)
产品合同：[PRINCIPLES.md](../../PRINCIPLES.md)

## 摘要

DistributedFs 使用 inode owner 管理普通可变文件的全局 dirty view。普通 write 返回表示数据已经被 owner 排序并在无故障运行时可见；它不必创建 Chunk 或 FileVersion。同步或后台 CommitTrigger 冻结一个写入前缀，ChunkStore 先满足 DurabilityPolicy，随后 Meta CAS 原子创建 FileVersion 并推进 `InodeRecord.head_version`。

## 外部语义

1. 普通 `write/pwrite` 返回后，无故障运行时后续读取必须看到该写入或全局顺序中更晚的覆盖写。
2. 普通 write 不保证 Node、磁盘或集群故障后的恢复。
3. `fdatasync` 提交文件数据及恢复数据所必需的 Chunk、ExtentMap、LayoutRoot、FileVersion、length 和 `head_version`。
4. `fsync` 在 `fdatasync` 基础上再同步 `mtime/ctime` 等完整 inode 属性。
5. 文件 `fsync` 不保证父目录项；目录项需要单独 `fsync(dir)`。
6. `O_DSYNC` 和 `O_SYNC` 分别把 fdatasync/full fsync 屏障放到每次 write 返回之前。
7. `close`、FUSE `flush` 和 FUSE `release` 不增加超出此前成功同步操作的 crash guarantee。
8. `fsync` 不 Pin、不 Publish、不创建 Alias 或 RootManifest。
9. write 越过 EOF、`O_APPEND` 和 `truncate/ftruncate` 在 inode owner 上串行更新活动视图的 `logical_length`；其返回只提供与普通异步 write 相同的无故障可见性合同，后续同步操作才提交新 length。

## 规范时间线

```text
user      open        write       read         fsync              ok       close
───────────●────────────●───────────●────────────●──────────────────●──────────●────>

node     session       dirty      overlay       freeze    chunk   commit    release
───────────●────────────●───────────●────────────●─────────●────────●──────────●────>

meta     lease / V7                                         V8
───────────●─────────────────────────────────────────────────●──────────────────────>
```

- `dirty`：owner 已分配 WriteSeq 并更新共享 DirtyExtentMap；head 仍是 V7。
- `overlay`：普通读取使用 V7 加 dirty overlay；固定版本读取只读指定版本。
- `freeze`：CommitBatch 冻结 `seq<=S`；更晚写入进入下一批。
- `chunk`：新 Chunk 已满足 DurabilityPolicy，但 head 仍可以是 V7。
- `V8`：Meta CAS 原子创建 V8 并推进 head；这是版本真正成立的时刻。
- `commit`：Node 以 V8 为 base，并删除被 V8 覆盖的 dirty 前缀。
- `ok`：同步调用可以向用户返回成功。

## 写入协调模型

### WriteLease

```text
WriteLease {
  inode_id
  owner_node_id
  owner_session_id
  lease_epoch
  expires_at
}
```

同一时刻只有当前 epoch 的 owner 可以提交该 inode 的新版本。远程 writer 和 dirty reader 使用缓存路由直接访问 owner；路由失效时刷新 Meta，不在每个 write/read 上访问 Meta。

### DfsWriteSession

```text
DfsWriteSession {
  session_id
  handle_id
  inode_id
  open_flags
  lease_epoch
  last_accepted_seq
  last_synced_seq
  error_cursor
}
```

Session 不拥有 dirty bytes，只表达一次 open 的 flags、水位和错误观察位置。

### InodeWriteState

```text
InodeWriteState {
  inode_id
  lease_epoch
  base_version_id
  next_seq
  visible_seq
  durable_seq
  committed_seq
  logical_length
  dirty_extents
  pending_error
  open_writers
}
```

它是 Node 上当前可变文件状态。`DirtyExtentMap` 叠加于 committed base；它不是持久 FileVersion。

### CommitBatch

```text
CommitBatch {
  inode_id
  lease_epoch
  reason
  through_seq
  expected_head_version
  expected_inode_revision
  frozen_dirty_extents
  target_length
  metadata_delta
  operation_id
}
```

CommitBatch 是临时对象。lease epoch 和 write sequence 用于运行时 fencing、等待和清理，不是 FileVersion 的必需字段。

## CommitTrigger

同步触发器：

- `fdatasync`；
- `fsync`；
- `O_DSYNC` write；
- `O_SYNC` write；
- Runtime Snapshot barrier。

后台触发器：

- dirty age 或 dirty bytes；
- Node 内存压力；
- 最后一个 writer 关闭；
- lease handoff；
- Node drain 或优雅退出。

普通 write、read、FUSE flush 和 release 不是强制版本提交点。后台 trigger 可以创建 FileVersion，但不向应用提供一个新的可依赖完成点。

## 提交协议

1. owner 为普通 write 分配 WriteSeq，更新 DirtyExtentMap 后返回。
2. trigger 冻结全局前缀 `through_seq=S`。
3. owner 对冻结范围生成 StagedChunk；StagedChunk 只存在于 ChunkStore 内部。
4. ChunkStore 完成 Finalize 和 DurabilityPolicy，返回 ChunkReceipt。
5. owner 构造新的 immutable ExtentMap、LayoutRoot 和 FileVersion。
6. owner 以 `expected_head + expected_revision + lease_epoch + operation_id` 执行一次 Meta CAS。
7. Meta 原子写入版本和新 head；旧 epoch 被拒绝。
8. owner 清理 `seq<=S` 的 dirty ranges；更晚写入留给下一批。
9. 同步 trigger 向等待者返回；后台 trigger 继续运行而不产生用户完成事件。

Chunk 成功但 CAS 失败不会产生半个可见版本。未引用 Chunk 进入幂等重试或安全期后的 Orphan GC。

## 并发合同

- 同一 inode 的普通写入由 owner 分配全局顺序；不同 inode 可以由不同 owner 并行处理。
- 重叠写按 WriteSeq 顺序覆盖。
- `O_APPEND` offset 由 owner 基于当前 logical length 原子分配。
- `write/pwrite` 越过 EOF 时，owner 将 logical length 推进到 `max(old_length, offset+size)`；中间未覆盖范围是 Hole。
- Shrink 立即裁掉新 EOF 后的 dirty ranges 并推进活动视图；Grow 只推进 logical length。两者都不保存独立操作日志。
- fsync 屏障覆盖调用者此前成功 write 对应的 WriteSeq，以及全局顺序中不晚于该水位的写。
- 同步期间到达的更晚写入不进入已冻结 CommitBatch。
- 多个同步等待者可以共享覆盖其水位的同一批次。
- 后续如需突破单 inode owner 的吞吐上限，必须通过独立 RFC 引入 range lease 或等价机制。

## FUSE 生命周期合同

- `flush` 可以因 `dup`、`fork` 和多个 fd 多次发生，必须幂等；它排空前端缓冲并报告已知后台错误，不代表最后关闭。
- `release` 在一个 open file description 的引用结束后释放 DfsWriteSession；它的错误不能作为应用可靠观察的数据提交结果。
- dirty data 属于 InodeWriteState，不随任一 DfsWriteSession 的删除而丢失。
- 最后一个 writer release 可以调度后台 writeback，但不等待就不能声明同步完成。

## 错误与恢复

| 故障点 | 恢复合同 |
| --- | --- |
| write 返回后、同步前 owner 故障 | 允许回退到最后 committed FileVersion |
| Chunk 未满足策略 | 不推进 head；同步调用重试、重配置或返回错误 |
| Chunk 满足策略、CAS 前故障 | Chunk 未引用；恢复时幂等重试或 Orphan GC |
| CAS 成功、响应丢失 | 按 operation id 查询或回放提交结果 |
| 旧 owner 提交 | lease epoch 校验拒绝 |
| 后台 writeback 失败 | 在 inode 记录 sticky error，并由后续 write/flush/sync 观察 |

本地单副本、同步 N 副本和异步补副本策略必须分别声明 fsync 返回时已经完成的副本数和覆盖的故障域。文件同步与目录项同步分别验收。

## RPC 合同

1. create/open 的冷路径可以使用一个组合 Meta 操作返回 inode、head 和 lease。
2. 稳态普通 write 不访问 Meta；本地 owner 是进程内调用，远程 owner 使用可批量的 Peer 数据请求。
3. dirty read 在 lease 路由缓存命中时不访问 Meta。
4. 一个 CommitBatch 只执行一次 FileVersion CAS，不按 FUSE WRITE 或 Chunk 数量提交 Meta。
5. R=1/R=N 只在 ChunkStore 以下分叉。
6. 多 Chunk 使用连接池、长连接、batch 和 pipeline，逻辑 Chunk 数不与短连接 RPC 数一一绑定。

## 不变量

1. 普通 write 不修改 committed FileVersion 或 `InodeRecord.head_version`。
2. write 返回前必须达到 owner 排序后的 `DIRTY_VISIBLE`。
3. ChunkReceipt 成立不等于新版本已经可见。
4. 只有 Meta CAS 可以推进 `InodeRecord.head_version`。
5. CAS 前的候选版本不能被普通读取、Snapshot 或 Pin 当成 committed version。
6. `fdatasync/fsync` 成功返回前必须覆盖调用者此前成功写入的水位。
7. `flush/release/close` 不隐式执行业务发布。
8. dirty data 不属于某个可被单独 release 的 handle。
9. 旧 lease epoch 不能提交新版本。
10. 文件 `fsync` 和目录 `fsync` 是不同合同。
11. 同一 inode 的 write、append、truncate 和同步屏障由 owner 串行归并到 `logical_length + DirtyExtentMap`，第一版不引入 FileMutation 日志或 Meta AppendReservation。
12. FileVersion CAS 必须原子提交最终 length 与 LayoutRoot；不增加独立 LengthRecord、LengthHint 或按 Chunk 查询 EOF 的 RPC。

## 模块合同

### Meta

- WriteLease 的 owner、epoch 和 expiry；
- InodeRecord、FileVersion 和 Head CAS；
- operation outcome 与 result-unknown 查询；
- Dentry 和目录同步合同。

### Node

- DfsWriteSession、InodeWriteState 和 DirtyExtentMap；
- owner routing、WriteSeq、append offset 和 read overlay；
- CommitBatch、后台 writeback 和 sticky error；
- ChunkStore、ChunkReceipt 与 FileVersion commit client。

### FUSE

- 把用户 write/read/fsync/fdatasync/close 映射为 Backend 合同；
- 保留 `datasync`、`O_DSYNC` 和 `O_SYNC` 信息；
- flush/release 只处理前端 drain 和 handle 生命周期；
- 实现并验证 `fsync(dir)`，未实现前明确返回不支持。

## 拒绝的方案

### 每次 write 创建 FileVersion

拒绝。它把 Meta 放进每个 write 热路径，放大版本、Extent、事务和 GC 成本。

### 每个 handle 私有 dirty view

拒绝。它不能自然提供跨 handle、跨 Node 的 write 后可见性，也无法定义多 writer 和 append 顺序。

### close 等同于 fsync

拒绝作为外部合同。实现可以在 close 时主动后台提交，但不得让应用依赖未显式同步的 crash durability。

### 后台只写 Chunk、不提交版本

第一版拒绝。它还需要持久化 DirtyExtentMap 和恢复 journal，不能释放 owner 的全部可变状态。

### 可变 Chunk 代替 FileVersion

拒绝。它会改变 RFC-0002 已接受的 immutable Chunk 数据基座和固定版本多源读取边界。

### FileMutation 操作日志与 Meta AppendReservation

拒绝作为第一版基础抽象。同一 inode 的 write、append、truncate 和 fsync 已由 owner 串行处理并立即归并到 InodeWriteState；保存操作日志不能增加顺序保证。Append offset 同样由 owner 原子分配，每次请求访问 Meta 会把小写热路径变成控制面瓶颈。

## 验收标准

- 普通 write 返回后，同 Node 和远程 reader 均能通过 owner 看到新内容，且 write 热路径不访问 Meta。
- owner 在未同步 write 后故障，恢复到最后 committed FileVersion，不产生半版本。
- `fdatasync`、`fsync`、`O_DSYNC` 和 `O_SYNC` 分别满足本文定义的完成合同。
- `dup` 产生的多次 flush 幂等，release 不承担唯一提交。
- 两个 Node 并发覆盖和 append 获得可解释的全局顺序。
- R=1/R=N 使用相同文件提交协议，只在 ChunkStore 以下分叉。
- Chunk durable、CAS 前故障、CAS response lost 和 stale lease 均有自动化故障验收。
- 新文件完成 `fsync(file)+fsync(dir)` 后，内容和名称都能在声明故障域内恢复。
