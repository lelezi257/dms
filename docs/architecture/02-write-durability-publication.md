    # 专题二：写入完成、持久化与可见性

状态：Accepted Design
实现状态：R=1 基础框架已实现；远端 owner、非阻塞 freeze 与目录同步未实现
专题入口：[架构设计专题](design-topics.md)
数据模型：[专题一](01-file-version-chunk-model.md) · [RFC-0002](../rfcs/0002-file-version-chunk-model.md)
规范合同：[RFC-0003](../rfcs/0003-write-visibility-durability.md)

## 1. 目标

本专题定义 POSIX 用户在 `open`、`write`、`read`、`fdatasync`、`fsync` 和 `close` 各时刻可以依赖什么，并将这些用户动作映射到 Node 的可变写入状态、不可变 Chunk 和 Meta FileVersion。Snapshot、Pin 和 Publish 使用已经提交的精确 FileVersion，不与普通文件同步混为一个操作。

核心边界是：

- 普通 `write` 返回表示数据已经进入 inode 的全局写入顺序，无故障运行时后续读取可见，但不保证故障后恢复；
- `fdatasync` 提交文件数据及恢复这些数据所必需的 Chunk、ExtentMap、LayoutRoot、FileVersion、length 和 `head_version`；
- `fsync` 在 `fdatasync` 基础上再同步 `mtime/ctime` 等完整 inode 属性；
- 文件同步不保证父目录项，目录项需要单独 `fsync(dir)`；
- `flush` 和 `release` 是 FUSE 生命周期回调，不是业务发布，也不替代同步合同；
- FileVersion 只在 Meta CAS 成功时成为新的 committed 版本。

## 2. 用户、Node 与 Meta 的总纲时间线

主线描述普通 write 后由用户显式调用 `fsync` 的过程。图中只保留用户动作和三个组件可观察到的关键状态，详细语义在图后定义。

```text
user      open        write       read         fsync              ok       close
───────────●────────────●───────────●────────────●──────────────────●──────────●────>

node     session       dirty      overlay       freeze    chunk   commit    release
───────────●────────────●───────────●────────────●─────────●────────●──────────●────>

meta     lease / V7                                         V8
───────────●─────────────────────────────────────────────────●──────────────────────>
```

版本变化只发生在 Meta 线上的 `V8`。`write`、`dirty`、`overlay`、`freeze` 和 `chunk` 都不会单独推进 `InodeRecord.head_version`。

### 2.1 `open` / `session` / `lease`

`open` 解析 Dentry 和 InodeRecord，取得当前 committed head V7，并获取或复用该 inode 的 WriteLease。Node 为本次 open 建立 `DfsWriteSession`。Meta 中的 V7 仍然是可恢复内容的唯一权威版本。

### 2.2 `write` / `dirty`

inode owner 为 write 分配单调递增的 WriteSeq，并把范围更新到 inode 共享的 `DirtyExtentMap`。`write` 返回时数据至少达到 `DIRTY_VISIBLE`：无故障运行时，同一 inode 的后续读取必须能够看到它。此时数据可以仍然只在 owner 内存中，Meta head 仍是 V7。

### 2.3 `read` / `overlay`

活跃 lease 存在时，其他 Node 将读取路由到 owner。owner 使用 `committed V7 + DirtyExtentMap` 生成读视图。固定版本读取继续直接读取指定 FileVersion，不混入 dirty overlay。

### 2.4 `fsync` / `freeze`

owner 在收到同步请求后冻结一个全局写入前缀 `through_seq=S`，形成 CommitBatch。之后到达的 write 取得更大的 WriteSeq，留给后续批次，不需要长时间阻塞整个 inode。

### 2.5 `chunk`

CommitBatch 把冻结的 dirty ranges 规范化为新 Extent 和不可变 ChunkObject。ChunkStore 完成 R=1、R=N 或其他 DurabilityPolicy，并返回满足策略的 ChunkReceipt。此时新数据已经达到介质和副本合同，但 Meta head 仍可能是 V7。

### 2.6 `V8` / `commit`

Node 使用当前 lease epoch、expected head、expected inode revision 和 operation id 执行一次 Meta CAS。事务原子写入新的 FileVersion、LayoutRoot/Extent 和 `InodeRecord.head_version=V8`。CAS 成功后，Node 以 V8 为新的 base，删除已被 V8 覆盖的 `seq<=S` dirty ranges；更晚的 dirty ranges 继续存在。

### 2.7 `ok`

`fsync` 返回成功表示 V8 已经提交并满足当前 DurabilityPolicy。`fdatasync` 的成功点相同，但完整 inode 属性合同较弱。同步调用返回前发生的 result-unknown 必须通过 operation id 查询或重放，不能盲目生成另一个版本。

### 2.8 `close` / `release`

用户调用 `close` 时，FUSE 可以调用一次或多次 `flush`，并在同一 open file description 的最后引用消失后调用一次 `release`。`flush` 排空前端请求并报告已知后台错误；`release` 删除 `DfsWriteSession` 和资源引用。它们不自动 Pin、Publish 或创建 RootManifest。

## 3. 可见性与持久化是两个维度

| 状态 | 无故障时其他 reader | Node 故障后恢复 | committed head |
| --- | --- | --- | --- |
| Session 私有前端缓冲 | 不保证 | 不保证 | V7 |
| `DIRTY_VISIBLE` | 通过 owner 可见 | 不保证 | V7 |
| `POLICY_DURABLE`、CAS 前 | 通过 owner 可见 | Chunk 存在但尚未被 head 引用 | V7 |
| `VERSION_COMMITTED` | 可从 V8 直接读取 | 按 DurabilityPolicy 恢复 | V8 |

普通 write 的可见性不能只存在于某个 handle 私有内存中。达到 `DIRTY_VISIBLE` 前，远程 writer 必须收到 owner 接受和排序的确认；Meta 不参与每个 write。

## 4. 哪些动作触发 FileVersion 提交

### 4.1 用户同步触发

| 触发 | 是否等待 | 完成合同 |
| --- | --- | --- |
| `fdatasync(fd)` | 是 | 数据和恢复数据所必需的所有索引、length、FileVersion 与 head |
| `fsync(fd)` | 是 | `fdatasync` 合同加完整 inode 属性 |
| `O_DSYNC` write | 每次 write 等待 | 等价于本次 write 被 fdatasync 屏障覆盖 |
| `O_SYNC` write | 每次 write 等待 | 等价于本次 write 被完整 fsync 屏障覆盖 |
| Snapshot barrier | Runtime 等待 | 先得到精确 FileVersionId，再 Pin 或构造 RootManifest |

多个同步等待者可以共享一个覆盖足够大 `through_seq` 的 CommitBatch。已经冻结的批次不吸收更晚写入；后续等待者在必要时创建下一批。

### 4.2 系统后台触发

后台 writeback 可以由 dirty age、dirty bytes、内存压力、最后一个 writer 关闭、lease handoff 或 Node drain 触发。后台提交允许创建内部 FileVersion，以释放 dirty memory 和 owner lease，但它不给应用增加一个可观察的同步保证。应用只能依赖成功的同步 API，而不能依赖“等待了足够久”。

### 4.3 不触发版本提交的动作

- 普通 `write/pwrite`；
- `read`；
- FUSE `flush` 本身；
- FUSE `release` 本身；
- 无脏数据的 `fdatasync/fsync`；
- 只修改 Copy Catalog、Repair、Rebalance 或 Cache 的操作。

## 5. POSIX 操作合同

| 用户操作 | Node 行为 | 返回时的保证 |
| --- | --- | --- |
| `write/pwrite` | owner 排序并更新 DirtyExtentMap | 无故障时后续读取可见；不保证故障恢复 |
| `read/pread` | 有活跃 dirty owner 时读取 base+overlay；固定版本读不混入 overlay | 返回某个明确读视图的数据 |
| `fdatasync` | 提交写入前缀 | 数据、恢复索引、length 和 head 可恢复 |
| `fsync` | `fdatasync` 加完整 inode 属性 | 文件内容和完整 inode 属性达到合同 |
| `close` | drain、release，必要时调度后台 writeback | 不增加超出此前成功同步操作的 crash guarantee |
| `fsync(dir)` | 提交 Dentry 与目录元数据 | 目录项达到持久化合同 |

新建文件若只执行 `fsync(file)`，不能据此保证父目录项在崩溃后存在。要求名称也持久时，应用必须再执行 `fsync(dir)`。当前 `fsyncdir` 尚未实现，必须明确返回不支持并作为实现缺口跟踪。

## 6. 核心运行时抽象

### 6.1 WriteLease

```text
WriteLease {
  inode_id
  owner_node_id
  owner_session_id
  lease_epoch
  expires_at
}
```

WriteLease 为活跃可写 inode 指定 owner，并用 epoch 拒绝旧 owner 的提交。不同 inode 可以分布在不同 Node；第一版同一 inode 的普通写入在 owner 上形成全局顺序。

### 6.2 DfsWriteSession

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

Session 只保存一次 open 的 flags、水位和错误观察位置，不拥有文件 dirty data。

### 6.3 InodeWriteState 与 DirtyExtentMap

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

`DirtyExtentMap` 是 committed FileVersion 上的内存 overlay。它合并覆盖写、支持 base+overlay 读取，并在 CommitBatch 中规范化为新的不可变 ExtentMap。它不是一个可持久引用的 Mutable FileVersion。

### 6.4 CommitBatch

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

CommitBatch 是一次提交的临时对象。`lease_epoch` 和 `through_seq` 用于运行时排序、fencing 和清理，不作为 FileVersion 必需字段。

### 6.5 StagedChunk

StagedChunk 保留为 ChunkStore 内部的临时构造状态：

```text
CommitBatch -> StagedChunk -> finalize -> ChunkObject
```

StagedChunk 不进入 Meta UML，不被 FileVersion、普通读取、Snapshot、Cache Seed 或 Repair 引用。

## 7. E2E Case

### 7.1 新文件、R=1、本地 owner

1. `open(O_CREAT)` 通过一次 Meta 操作建立 Dentry、InodeRecord 和 WriteLease，返回空 head 与 lease epoch。
2. 多次 FUSE WRITE 被 owner 排序为 DirtyExtent；每次 write 返回前不访问 Meta。
3. 其他 handle 读取时由 owner 把 dirty overlay 叠加到空 base。
4. `fsync` 冻结调用前已接受的写入前缀。
5. ChunkStore 在本地执行临时写、文件同步、rename、目录同步和校验，返回 R1 ChunkReceipt。
6. Node 构造 ExtentMap、LayoutRoot 和 V1，并执行一次 Meta CAS。
7. CAS 成功后 fsync 返回；后续 close 只释放会话。

### 7.2 V7 中间覆盖 4 KiB

普通 write 只增加覆盖范围的 DirtyExtent。同步时仅为 patch 生成新 Chunk，V8 的 ExtentMap 复用 V7 未修改范围。大量小覆盖造成的 Extent 碎片由专题四的 Compaction 处理，不在 write 热路径重写整个文件。

### 7.3 R=3

文件层流程与 R=1 相同，分叉只发生在 `ChunkStore::put`。本地 owner 作为优先 head 时，数据沿 A→B→C 流水传输；只有满足 R=3 的 ChunkReceipt 才允许 V8 CAS。若只等待本地副本，应使用 `LOCAL_DURABLE_ASYNC_REPLICA`，不能声明 R=3 已完成。

### 7.4 跨 Node 多 writer

Node A 持有 lease，Node B 的 write 通过长连接发送给 A。A 分配 WriteSeq；重叠范围按该顺序覆盖，`O_APPEND` 的 EOF offset 也由 A 原子分配。远程 reader 在 lease 活跃期间路由到 A。B 的 fsync 是一个 `through_seq` 屏障，由 A 统一生成 FileVersion。

### 7.5 close without fsync

```text
user      open       write        close                  crash
───────────●───────────●────────────●──────────────────────●────────>

node     session      dirty       release      background
───────────●───────────●────────────●─────────────●─────────────────>

meta     lease / V7                              V8?
───────────●──────────────────────────────────────○──────────────────>
```

空心 `V8?` 表示后台提交可能完成，也可能尚未完成。close 成功不允许用户推断 V8 已经存在；立即故障时可以恢复到 V7。

### 7.6 `O_SYNC`

```text
user      open       write                                ok
───────────●───────────●────────────────────────────────────●───────>

node     session      dirty      freeze      chunk       commit
───────────●───────────●──────────●────────────●────────────●───────>

meta     lease / V7                              V8
───────────●──────────────────────────────────────●─────────────────>
```

这里的 `ok` 是 write 返回。返回前已经完成 V8 提交和完整 inode metadata 合同。

## 8. RPC 与数据 Hop

统一计数：FUSE callback 和进程内调用不算 RPC；request/response 是一次逻辑 RPC exchange；完整 payload 跨一条网络连接是一次 data hop；ACK 是控制消息。

| 场景 | Meta RPC | Peer RPC / data hop |
| --- | --- | --- |
| 新文件 `CreateAndOpenWrite` | 1 | 0 |
| 本地 owner 普通 write | 0 | 0 |
| 远程 writer 普通 write | 0 | 1 个可批量 Peer exchange / 1 hop |
| 远程 dirty read | cache 命中时 0 | 1 个 Peer read exchange |
| R=1 fsync | 1 次 FileVersion CAS | 0 |
| R=3 fsync，本地是 head | 1 次 FileVersion CAS | 每个新 Chunk 两个 payload hop，wire 层应批量和流水 |
| 无脏数据的 fsync | 0，必要时只同步属性 | 0 |
| flush / release | 0 | 只在仍有前端待发送请求时 drain |

冷路径 writable open 应把 inode、当前 FileVersion 和 lease 放在一个逻辑 Meta 操作中。稳态 write 不访问 Meta；placement、lease 和路由按 inode 或批次缓存，过期时刷新。

## 9. 故障与用户合同

| 故障点 | 用户已获得的保证 | 恢复结果 |
| --- | --- | --- |
| write 返回后、同步前 owner 故障 | 无故障时可见 | 允许回退到最后 committed 版本 |
| Chunk 未满足策略 | fsync 尚未返回 | 重试、重配置或返回错误；不推进 head |
| Chunk 已满足策略、CAS 前故障 | fsync 尚未返回 | Chunk 成为 orphan 或被幂等重试引用 |
| CAS 成功、响应丢失 | 结果未知 | 按 operation id 查询或回放成功结果 |
| 旧 lease owner 提交 | 无 | Meta 按 epoch 拒绝 |
| 后台 writeback 失败 | 普通 write 只获得可见性合同 | 记录 sticky error，在后续 write/flush/sync 报告 |

`R1_LOCAL` 只覆盖本地介质仍可用的重启恢复，不覆盖永久磁盘丢失；`R3_SYNC` 才提供声明范围内的 Node/磁盘故障容忍；异步副本策略必须明确返回时仍存在的单点风险。

## 10. 与 3FS 的关系

3FS 的 buffered write 可以只进入 inode 共享缓冲；`flushBuf` 将范围切成 WriteIO 并完成可变 Chunk 的 CRAQ update/forward/reverse commit；`fsync` 再同步 inode length/mtime。它证明了数据提交与 Meta 属性同步应分层，也证明普通 write 不需要每次访问 Meta。

AFS 不直接复制其状态机：3FS 在 chain head 串行同一可变 Chunk，而 AFS 的 ChunkObject 提交后不可变，并通过 FileVersion CAS 发布写入前缀。AFS 第一版使用 inode owner 组合全局 dirty view；3FS 的 FUSE `flush` 当前直接调用完整 fsync，AFS 则保持 flush、同步和 release 三个合同分离。

可吸收的原则包括：Meta 脱离稳态数据路径、共享 inode 缓冲、完成水位、稳定 request id、可靠重试、长连接、批量 pipeline，以及小数据 inline/大数据 buffer descriptor 或流式传输。

## 11. 当前实现状态与差距

R=1 基础框架已经把 dirty data 从 handle 移到 inode 共享的 `InodeWriteState/DirtyExtentMap`。`DfsWriteSession` 只保留 open flags、lease epoch、同步水位和错误游标。Meta 已实现组合 `OpenWrite`、内部 lease acquire、公开 renew，以及 `WriteLease` 的 `owner_node_id + owner_session_id + lease_epoch` 围栏；FileVersion commit 同时校验 expected head/revision。FUSE `write/flush/fdatasync/fsync/release` 已按本专题分离，`O_DSYNC/O_SYNC` 已接通；`fdatasync` 已提交 dirty data 后，后续无新数据的 `fsync` 使用同一 lease 围栏执行 metadata-only CAS，不创建空 FileVersion。Node 定时执行后台 writeback，并在优雅退出时 drain。真实 Linux FUSE 用例已验证另一个 handle 在同步前读取 dirty overlay，以及 `fdatasync(V1) → 覆盖写 → fsync(V2) → close/reopen`。

当前边界如下：

1. `OpenWrite` 能返回远端 owner 身份，但 Node 间 write/read/sync 转发尚未接入；当前非本机 owner 明确返回不支持。
2. Commit 期间只持有对应 inode 的锁，不持有全局 handle table；尚未实现冻结前缀后让更晚 write 并行进入下一批。
3. `DirtyExtentMap` 已表达覆盖范围，但 R=1 Commit 暂时仍把完整当前文件物化为一个 Chunk；Patch Chunk、Extent 树和 compaction 属于专题四。
4. 后台失败会记录 inode sticky error，并由每个已打开 writer 的错误游标观察；故障注入、错误清除和重启恢复矩阵仍需补齐。
5. DFS 只有 R=1 本地 ChunkStore；R=N ChunkReceipt 与 result-unknown 由专题三继续设计和实现。
6. `fsync(dir)` 尚未实现，不能声明新建文件名已经满足崩溃恢复合同。

源码入口见 [`dfs.rs`](../../src/node/vfs/dfs.rs)、[`chunk.rs`](../../src/node/chunk.rs)、[`fuse.rs`](../../src/node/fuse.rs)和 [`meta/dfs.rs`](../../src/meta/dfs.rs)。

## 12. 后续专题输入

- 专题三承接 `freeze → chunk → policy durable`，定义 R=1/R=N 状态机、重配置和 result-unknown；
- 专题四定义 StagedChunk、Patch Chunk、Extent 合并、Compaction 和本地恢复；
- 专题五定义 dirty/committed length、append、truncate 与 Snapshot barrier；
- 专题六定义幂等、错误账本、wire batching、P2P、Cache、Spill 和高性能数据路径。
