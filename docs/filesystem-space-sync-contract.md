# DMS Native Filesystem 空间管理与同步合同设计

> 状态（2026-09-16）：M1.5 已在 `feat/native-filesystem` 完成实现、端到端验证和
> 人工 Review；本阶段代码提交到特性分支，不合入 `main`。

## 1. 本阶段要解决的用户问题

M1.4 已经能正确表达文件内容、属性和容量查询，但空间与同步语义仍有两个缺口：

1. 应用调用 `fallocate` 时，DMS 必须区分“只读为零的 HOLE”和“已经保证给该文件使用的预留空间”。
2. 应用调用 `flush/fsync/fdatasync` 时，DMS 必须明确成功究竟保证了什么，不能因为当前是 write-through 就在 FUSE 层直接返回成功。

用两个具体例子表示：

```text
fallocate(fd, KEEP_SIZE, 0, 64 MiB)
  期望：文件 size 不变；未来写入已预留范围不再因为 DMS 容量不足而失败。
  不能：只创建一个 HOLE。HOLE 不占物理空间，无法兑现上述保证。

pwrite(fd, "abc", 0)
fsync(fd)
  期望：返回条件由挂载时声明的 durability 决定。
  当前 local-memory：内容版本和 Meta WAL 已提交，Node 内存仍是易失介质。
  不能：把 local-memory 成功描述成主机掉电后仍可恢复。
```

本阶段继续使用 write-through，不引入 writeback、dirty page 或后台刷写队列。

## 2. 实现后的源码事实

| 当前路径 | 已完成 | 保持的边界 |
| :--- | :--- | :--- |
| `SharedFileOperations` | `write/fallocate/punch-hole/truncate/flush/sync` 都从同一文件业务层进入 DataCore 与 Meta | 继续 write-through，不引入 dirty page/writeback |
| Meta commit + journal | inode、exact content binding、属性、reservation 增减和失效事件由一次 commit/journal 原子发布 | WAL 只保护元数据；local-memory payload 不伪装成掉电可恢复 |
| `VersionLayout` | DATA 与 HOLE 分离；打洞只生成 HOLE overlay，不分配零 Block | Reservation 不进入 Extent 或读布局 |
| `ArenaManager` | Reservation 的申请、消费、恢复、释放均在唯一 Arena owner 中；相同 identity 重试幂等 | 首版不实现 quota 与跨 Node 持久 reservation |
| FUSE adapter | 覆盖 mode 0、`KEEP_SIZE`、`PUNCH_HOLE + KEEP_SIZE`、flush、fsync/fdatasync、fsyncdir | 其它 fallocate mode 稳定返回 `EOPNOTSUPP` |
| `O_SYNC/O_DSYNC` | write 成功后复用同一个文件 sync 业务入口 | 当前 local-memory + write-through 下不额外制造 Meta commit |

这说明跨层合同已经真正落到 FUSE、Node、Arena、Meta 和恢复路径，而不是只在
`fuse.rs` 增加几个成功返回的 handler。

## 3. 三个不能混淆的概念

| 概念 | 含义 | 是否占用物理空间 | 所属 owner |
| :--- | :--- | :--- | :--- |
| HOLE | 文件逻辑范围读为零，没有数据 Block | 否 | DataCore `VersionLayout` |
| DATA | 已写入的不可变 Block，由 Extent 引用 | 是 | DataCore / Arena |
| Reservation | 给未来 DATA 写入保留的容量承诺 | 是，但尚不是 Block | DataCore / Arena；Filesystem 只持有关联 |

Reservation 不是新的 Extent 类型。文件读布局仍只有 DATA 与 HOLE；reservation 只回答
“将来覆盖这个范围时是否已经有空间保证”。把 reservation 塞进 Extent 会让读路径、
副本定位和空间承诺三种职责互相污染。

## 4. 对外语义决策

### 4.1 `fallocate` 首版支持矩阵

| Linux mode | 首版语义 | 结果 |
| :--- | :--- | :--- |
| `0` | 预留 `[offset, offset+length)`；若越过 EOF，原子扩大 size；未写范围读零 | 支持 |
| `KEEP_SIZE` | 预留范围，但不改变 size | 支持 |
| `PUNCH_HOLE \| KEEP_SIZE` | 指定范围改为 HOLE；size 不变；旧 Block 按既有安全回收规则退休 | 支持 |
| `ZERO_RANGE`、`COLLAPSE_RANGE`、`INSERT_RANGE`、`UNSHARE_RANGE` | 首版不承诺 | `EOPNOTSUPP` |
| 其它组合、负 offset/length、零 length、范围溢出 | 非法参数 | `EINVAL` / `EFBIG` |

打洞不能先释放旧 Block 再提交新版本。正确顺序是先构造新布局、由 Meta 原子发布，
然后旧版本不再被 View 引用时再走既有 Block 退休流程。这样失败不会暴露半个洞。

### 4.2 `write`、`flush`、`fsync` 的边界

| 操作 | 首版返回条件 | 明确不代表 |
| :--- | :--- | :--- |
| `write` | 新 payload 已安装在 Node，Meta 已原子发布 exact version/size/mtime | 不代表主机掉电后 payload 可恢复 |
| `flush` | 该 open handle 已知的延迟错误已返回；write-through 下没有待提交 dirty bytes | 不是 `fsync`；不能当最终 close 或全局屏障 |
| `fdatasync` | 截止调用时，该 handle 的内容与正确读取内容所必需的 size/binding 已达到配置 durability | 不强制同步无关 mode/owner/time 更新 |
| `fsync` | 截止调用时，该文件已发布的内容和元数据都达到配置 durability | 不得静默把 stronger policy 降成 local-memory |
| 目录 `fsync` | 目录项及关联 inode mutation 已达到配置 durability | 不替代文件 payload 的 `fsync` |

`flush` 可能因 `dup/fork` 被调用多次，也可能根本不调用，因此必须幂等，不能用它判断
“最后一个 fd 已关闭”。真正每次 open 恰好一次的生命周期结束仍是 `release`。

### 4.3 local-memory 下 `fsync` 为什么仍可成功

DMS 当前公开且真实实现的 durability 是 `local-memory`。它与 `tmpfs` 一样是明确的易失
介质合同：同步成功表示状态已经到达该挂载声明的介质边界，不表示机器掉电后仍存在。

因此首版规则是：

- 挂载必须明确暴露 `local-memory`，不能把它命名成 durable/stable storage。
- `fsync/fdatasync` 在该策略下等待 write-through 提交与 Meta WAL 顺序点，然后成功。
- 用户若配置 `memory-copies:N`、`local-disk` 或 `object-store`，而实现尚未提供对应屏障，
  必须在挂载或调用边界返回不支持，不能降级。
- 后续增加更强策略时复用 DMS 已有 durability 配置，不在 Filesystem 再发明一套枚举。

同一原则也约束 reservation：local-memory 预留只在持有它的 Node incarnation 存活期间
有效；该 Node 故障后，Meta 必须使旧 reservation 失效，不能让后续写误以为仍有空间。
只有对应 durability 的多副本或稳定介质 reservation 落地后，才能承诺跨 Node 故障仍保留
预分配保证。这不是 `fallocate` 的特殊降级，而是当前整个易失内存文件系统的故障边界。

### 4.4 `O_SYNC/O_DSYNC`

`O_SYNC` 与 `O_DSYNC` 不建立另一条写路径。open handle 保存既有 flags；每次 write 原子发布
成功后，分别执行与 `fsync`/`fdatasync` 相同的屏障，再向内核返回。当前 write-through +
local-memory 下屏障通常只确认同一提交序号，没有后台等待；将来 durability 变强时调用点不变。

## 5. 分层与目录设计

```text
server/src/
├─ filesystem/
│  ├─ model.rs                 现有 inode/dentry/content binding
│  └─ space_sync.rs            本阶段跨层领域值；不持有状态
├─ node/
│  ├─ filesystem/
│  │  ├─ fuse.rs               解码 Linux flags/errno；不实现空间算法
│  │  ├─ open_handles.rs       flags、handle 顺序点、延迟错误
│  │  └─ shared.rs             fallocate/sync 业务编排与 CAS 重试
│  ├─ data_core.rs             预留、打洞候选、sync barrier 的进程内入口
│  ├─ version_layout.rs        继续只处理 DATA/HOLE 布局
│  └─ arena_manager.rs         Reservation/Slot/Block 物理生命周期唯一 owner
└─ meta/
   ├─ filesystem_catalog.rs    inode 与 reservation 关联、幂等结果
   ├─ metadata_journal.rs      提交顺序与持久性边界
   └─ runtime.rs               单 actor turn 原子发布与 rollback

protocol/proto/dms/v1/
└─ filesystem_meta.proto       只增加跨 Node→Meta 必需的请求；FUSE flags 不上 wire
```

不会新增 `SpaceManager`、第二个 allocator、Filesystem 专用 Extent 或新的后台 runtime。

## 6. 已落盘的接口与真实调用点

源码位置：`server/src/filesystem/space_sync.rs`。

```rust
enum FileSyncMode {
    DataOnly,
    DataAndMetadata,
}

struct SpaceRange {
    offset: u64,
    length: u64,
}

enum FileSpaceMutation {
    Preallocate { keep_size: bool },
    PunchHole,
}

struct FileSpaceMutationRequest {
    range: SpaceRange,
    mutation: FileSpaceMutation,
}
```

`SpaceRange` 与请求字段保持私有，只能通过经过长度/溢出校验的构造函数创建。领域类型已经
被真实 FUSE 路径使用，不存在 `dead_code` 豁免。

这几个类型刻意没有出现：

- `libc::FALLOC_FL_*`：Linux flags 在 `fuse.rs` 转换后消失。
- `protobuf`：领域层不依赖传输 DTO。
- `DurabilityPolicy` 新副本：复用已有配置合同。
- Filesystem 自己的 allocator：物理 reservation identity 直接复用幂等 operation id，
  不建立第二套身份生成器。

实际方法入口收敛为：

```rust
SharedFileOperations::mutate_space(handle, FileSpaceMutationRequest)
SharedFileOperations::flush(handle)
SharedFileOperations::sync(handle, FileSyncMode)
SharedFileOperations::sync_directory(inode)

DataCoreHandle::reserve_file_space(...)
DataCoreHandle::consume_file_space(...)
DataCoreHandle::restore_file_space(...)
DataCoreHandle::release_file_space(...)
DataCoreHandle::prepare_punch_hole(...)
```

当前 write-through 的 sync barrier 已被前序提交满足，因此文件 `sync` 只验证 handle，目录
`sync_directory` 通过既有 inode cache/resolve 路径确认目录身份；二者都不会再发起一次 Meta
mutation/commit。provider、SHM、gRPC、Region/Slot 选择继续封装在 DataCore 以下。

关键代码阅读顺序：

1. `server/src/node/filesystem/fuse.rs`：Linux flag 解码、stable errno、主 span 与失败日志。
2. `server/src/node/filesystem/shared.rs`：空间操作、CAS 重试、reservation 与提交结果编排。
3. `server/src/node/data_core.rs`、`server/src/node/runtime.rs`：进入唯一 Node actor/Arena owner。
4. `server/src/node/arena_manager.rs`：物理容量 admission、幂等 reservation 生命周期与统计。
5. `server/src/meta/runtime.rs`：校验 Node incarnation，在同一 journal record 中发布 inode、
   content version、reservation 增减和 revoke。
6. `protocol/proto/dms/v1/filesystem_meta.proto`：只携带 Node→Meta 必需的 reservation 变化；
   FUSE mode 不进入 wire。

## 7. 三条 E2E 主流程

### 7.1 KEEP_SIZE 预留后写入

```sequence
participant U as 用户进程
participant F as FUSE adapter
participant N as SharedFileOperations
participant D as DataCore/Arena
participant M as MetaState
U ->> F: fallocate(KEEP_SIZE, 0, 64 MiB)
F ->> N: mutate_space(Preallocate keep_size=true)
N ->> D: reserve_file_space(64 MiB)
D -->> N: reservation（不是 Block）
N ->> M: 原子关联 reservation 与 inode
M -->> N: 新 inode revision
N -->> U: 成功；size 不变
U ->> N: pwrite(已预留范围)
N ->> D: reservation 转为 DATA allocation
N ->> M: 原子发布新版本并扣减 reservation
M -->> U: write 成功
```

若 Meta 明确拒绝提交，DataCore 释放本次新增 reservation；若响应状态未知，Meta client 使用
同一 operation id 重试，不能释放一份可能已经提交的 reservation。写入预留范围时，Arena
先生成可逆 consumption；Meta 明确拒绝才恢复，提交成功才确认扣减。

### 7.2 打洞

```sequence
participant U as 用户进程
participant F as FUSE adapter
participant N as SharedFileOperations
participant D as DataCore
participant M as MetaState
U ->> F: fallocate(PUNCH_HOLE|KEEP_SIZE, range)
F ->> N: mutate_space(PunchHole)
N ->> D: prepare_punch_hole(当前 exact version, range)
D -->> N: 新 VersionCandidate（range=HOLE）
N ->> M: CommitFilesystemVersion(CAS)
M -->> N: 新 exact version + revoke
N ->> D: finalize；旧 Block 延迟退休
N -->> U: 成功；size 不变，range 读零
```

### 7.3 `fsync` 与目录 `fsync`

```sequence
participant U as 用户进程
participant F as FUSE adapter
participant N as SharedFileOperations
U ->> F: fsync(file)
F ->> N: sync_file(DataAndMetadata)
N ->> N: 验证 handle；此前 write-through 已越过当前屏障
N -->> U: 成功
U ->> F: fsync(directory)
F ->> N: sync_directory(DataAndMetadata)
N ->> N: 通过 cache/resolve 确认目录；不新增 mutation
N -->> U: 成功
```

在当前 local-memory + 同步 Meta WAL 下，barrier 已在 write/mutation 返回前完成，因此
`flush/fdatasync/fsync/fsyncdir` 不产生新的 Meta commit。接口仍必须存在，因为 future
durability、并发提交顺序和错误传播都需要明确的截止点。

## 8. 容量失败与回滚

| 失败点 | 对外错误 | 必须回滚/保留的状态 |
| :--- | :--- | :--- |
| Arena 无法建立 reservation | `ENOSPC` | 不改 inode、size、layout |
| quota 拒绝 | `EDQUOT` | 不占 Arena；首版 quota 未实现时不能伪造该错误 |
| reservation 成功、Meta 关联失败 | 原因对应 errno | 释放本次 reservation |
| 持有 local-memory reservation 的 Node incarnation 失效 | 后续依赖该保证的写返回稳定失效/容量错误 | Meta 删除旧关联；不能把已丢失 reservation 当作仍有效 |
| punch-hole prepare 成功、Meta CAS 冲突 | 重试或 `EAGAIN` | 旧 Current 与旧 Block 不变 |
| Meta 已提交但响应丢失 | 复用 operation id 查询/重试 | 不能重复预留或重复释放 |
| sync barrier 失败 | `EIO` 或稳定映射错误 | 已发布版本仍存在；错误必须留在 handle 供 flush/fsync 观察 |

`ENOSPC` 表示物理容量不足；`EDQUOT` 表示租户/工作区额度不足。二者即使最终映射到相似
恢复动作，也不能混成同一个内部原因。

## 9. 可观测性边界

- Metrics：业务层增加固定低基数 `fallocate/flush/sync`，FUSE callback 维度记录
  `fallocate/flush/fsync/fsyncdir`，Arena 输出 reservation bytes/count；不把 inode/path 当 label。
- Trace：每个用户操作一个主 span；DataCore reserve、Meta commit、sync barrier 是子 span；
  不给每个 4 KiB Extent 建 span。
- Log：只记录失败、rollback 失败和不支持的 durability；不记录成功热路径，不输出文件内容。

## 10. 已完成的验收矩阵

### 10.1 单 VM

1. `fallocate` mode 0 与 `KEEP_SIZE` 的 size、读零和 Arena reserved bytes/count 正确。
2. 预留范围内写入会扣减 reserved bytes，远端 Node 第一次读取立即看到新数据。
3. `PUNCH_HOLE|KEEP_SIZE` 后 size 不变，指定 range 跨 Node 读取为零。
4. 注入 ENOSPC 后稳定返回 errno 28，size、logical bytes、reserved bytes/count 全部不变。
5. `flush/fdatasync/fsync/fsyncdir` 走真实 SharedFileOperations，sync-only Meta commit 增量为 0。
6. `O_SYNC/O_DSYNC` 写返回后，远端 Node 立即看到完整内容。
7. Meta 重启后相同预留不重复占容量；Node owner 重启后旧 incarnation reservation 被 fencing。

### 10.2 三 VM

1. A/B 分别运行 FUSE Node，C 独立运行 Meta；所有用户语义与单 VM相同。
2. A 预留/写入/打洞后，B 第一次读取立即看到相同 size、DATA/HOLE 与内容。
3. Meta 重启后 reservation identity 和容量统计保持不变。
4. Node A 重启产生新 epoch 后，Node B 能覆盖旧 owner 的预留范围，不把旧承诺当成有效容量。
5. typed FUSE、Filesystem、Arena 与 Meta 指标均由真实进程导出，评价器结果为 PASS。

### 10.3 实测结果与证据

| 场景 | 单 VM | 三 VM | 验收结论 |
| :--- | ---: | ---: | :--- |
| mode 0 预留并扩展 | 3.708 ms | 5.704 ms | PASS |
| KEEP_SIZE 预留 | 4.272 ms | 6.247 ms | PASS |
| 预留内写入 4 KiB | 2.578 ms | 5.391 ms | PASS |
| 打洞 4 KiB | 4.059 ms | 8.935 ms | PASS |
| fsync | 0.043 ms | 0.049 ms | PASS；额外 Meta commit 为 0 |
| `O_SYNC` / `O_DSYNC` 写 | 1.638 / 1.387 ms | 1.343 / 1.293 ms | PASS；复用同一 sync 合同 |
| ENOSPC | 0.984 ms | 1.423 ms | PASS；无半状态或容量泄漏 |

这些数字是正确性 E2E 的单次观测，用于暴露数量级和请求放大，不作为稳定性能基线。
权威机器合同是 `benchmarks/whitebox/filesystem-space-sync-contract.json`；单 VM 证据位于
`evidence/2026-09-16-filesystem-space-sync-114651/`，三 VM 证据位于
`evidence/2026-09-16-filesystem-space-sync-3vm-final/`。

## 11. 本轮实现 Review 重点

1. Reservation 是否始终只是 Arena 容量承诺，没有进入 Extent、读路径或第二套 allocator。
2. 未知提交结果是否始终复用 operation id，且没有提前释放可能已提交的 reservation。
3. 预留的申请、消费、恢复、释放是否与 Meta 增减在成功/明确失败边界上严格对应。
4. sync 路径是否复用 write-through 已完成的屏障，没有制造额外 Meta commit 或成功热路径日志。
5. Node epoch/lease 是否能 fencing 旧 local-memory reservation，且重启恢复没有双重计费。
6. 新增 metrics/trace/log 是否低基数、失败导向，并覆盖真实 FUSE 而不是只覆盖单元函数。

## 12. 语义依据

- [Linux `fallocate(2)`](https://man7.org/linux/man-pages/man2/fallocate.2.html)
- [libfuse `flush/fsync` 合同](https://libfuse.github.io/doxygen/structfuse__operations.html)
