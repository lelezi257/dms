# DMS 原生 Filesystem 文件尺寸语义代码导读

## 1. 本轮结果

本轮把文件尺寸相关的 POSIX 基础语义接入到现有 Native Filesystem 主链：`truncate` 缩短与扩展、`open(..., O_TRUNC)`、越过 EOF 的 `pwrite`、并发 `O_APPEND`，都通过同一条 `DataCore prepare → Meta filesystem commit → DataCore finalize` 的 write-through 路径发布。

这次不是新增一套文件内容模型。文件仍然复用 DMS 既有 `ObjectVersion → VersionLayout → Extent → Block`：

- 文件层负责 inode、size、mtime、open handle 和用户文件偏移。
- DataCore 负责把一次文件内容版本表达成 DATA/HOLE Extent 布局。
- Node 负责本地 Block、Region/Arena、Peer 拉取和读路径 materialize。
- Meta 负责唯一权威提交：对象版本、inode 精确绑定、文件 size、mtime、失效事件和幂等结果在一条 journal record 中一起发布。

最终单 VM 与三 VM 真实验证均通过。证据显示：`truncate` 扩展只增加逻辑 hole，不新增全零 Block；越过 EOF 的 `pwrite` 只为用户真实写入的尾段分配 4 bytes；`O_TRUNC` 返回前已经发布 size=0；16 个并发 `O_APPEND` 写入可以通过 CAS 重试全部提交；跨 Node 与重启后都能读到正确文件尺寸和内容绑定。

## 2. 一个用户文件如何落到 DMS 抽象

用户执行：

```text
fd = open("/cases/a.txt", O_RDWR | O_CREAT)
write(fd, "abcdef")
pwrite(fd, "X", 1, 2)
truncate("/cases/a.txt", 10)
```

DMS 内部不是把文件当成一个可原地修改的 byte array，而是每次成功写入发布一个新的不可变内容版本。

| 层 | 抽象 | 例子里的值 | 作用 |
| :--- | :--- | :--- | :--- |
| Filesystem | Dentry | `("/cases", "a.txt") → inode 101` | 把路径名解析到 inode；rename 只改目录关系，不移动内容对象 |
| Filesystem | Inode | `inode=101, size=10, revision=N` | 承载 POSIX 属性、文件大小和当前内容绑定 |
| Filesystem | FileContentBinding | `object_key="fs/content/101", exact_version=3` | 明确 inode 当前指向哪个对象版本，不读取含糊的 Current |
| DataCore | VersionLayout | `logical_length=10, extents=[...]` | 描述这个版本的逻辑字节如何从 Extent 拼出来 |
| DataCore | DATA Extent | `[0..2) → Block B1[0..2)` | 逻辑范围来自某个不可变 Block 的一段 |
| DataCore | HOLE Extent | `[6..10) → zero` | 逻辑范围读出来是零，但没有 Block、副本、Arena 分配或网络传输 |
| Node | Block | `B1="abcdef"`、`B2="X"` | 不可变 bytes 身份；可以被多个版本的 Extent 复用 |
| Node | Region/Allocation | `B1`、`B2` 在本地内存中的物理位置 | 只负责内存定位与生命周期，不表达文件语义 |

## 3. DATA Extent 和 HOLE Extent

文件扩展和越过 EOF 写入会产生“空洞”。空洞是逻辑零字节，不是全零数据块。

```text
初始写入 "abcdef"

Version 1, size=6
[0..6)  DATA  -> Block B1[0..6)  "abcdef"

pwrite(offset=2, bytes="X")

Version 2, size=6
[0..2)  DATA  -> Block B1[0..2)  "ab"
[2..3)  DATA  -> Block B2[0..1)  "X"
[3..6)  DATA  -> Block B1[3..6)  "def"

truncate(size=10)

Version 3, size=10
[0..2)   DATA  -> Block B1[0..2)
[2..3)   DATA  -> Block B2[0..1)
[3..6)   DATA  -> Block B1[3..6)
[6..10)  HOLE  -> zero
```

这里的关键点是：

- `Block` 是真实 bytes 的身份。
- `Extent` 是逻辑文件范围到真实 bytes 或逻辑 zero 的映射。
- HOLE Extent 不引用 Block，Meta 不要求副本 proof，Node 不分配全零内存，读路径本地填零。
- 旧协议里未填写 kind 的 Extent 仍按 DATA 解释；只有显式 `EXTENT_KIND_HOLE` 才是 sparse hole。

协议字段落点：

| 文件 | 字段 | 含义 |
| :--- | :--- | :--- |
| `protocol/proto/dms/v1/node_meta.proto` | `ExtentRecord.kind` | Meta/Node 之间明确区分 DATA 与 HOLE |
| `protocol/proto/dms/v1/types.proto` | `PayloadTarget.zero` | Worker/SDK 普通读路径遇到 hole 时返回零段描述，不传输 payload |

## 4. 写入主流程

```sequence
participant U as 用户 / Kernel
participant F as Filesystem
participant D as DataCore / Node
participant M as Meta
participant W as 其他 Node
U ->> F: write / pwrite / truncate
F ->> F: 解析 open handle 与 inode
F ->> D: prepare 文件候选版本
D ->> D: 构造 DATA/HOLE Extent，必要时分配新 Block
D -->> F: PreparedObjectVersion
F ->> M: CommitFilesystemVersion(expected inode/object version)
M ->> M: 校验 Extent 与副本 proof，append journal，再 apply
M ->> W: Watch invalidate filesystem binding
W ->> W: 先删除 BindingCache
W -->> M: ACK
M -->> F: 新 inode snapshot + exact version
F ->> D: finish prepared blocks
F -->> U: 操作成功
```

write-through 的含义是：用户看到写成功时，Meta 已经发布了新 inode size、内容版本绑定和失效事件；当前没有 dirty page、后台写回或延迟错误语义。未来如果引入 write-back，必须单独定义 `fsync`、崩溃恢复、内存压力和错误延迟规则。

## 5. 四个文件尺寸 Case

### 5.1 truncate shrink

用户把文件从 8 bytes 缩短到 5 bytes。

```text
旧布局：
[0..8) DATA -> Block B1[0..8)

新布局：
[0..5) DATA -> Block B1[0..5)
```

Node 不读回完整文件，也不复制前 5 bytes。它只裁剪 VersionLayout 的最后一个 Extent，然后由 Meta 原子发布新的 inode size 和 exact version。未再被任何保留版本引用的旧 Block 后续由回收流程处理。

### 5.2 truncate grow

用户把文件从 3 bytes 扩展到 1 MiB+7。

```text
旧布局：
[0..3) DATA -> Block B1[0..3)

新布局：
[0..3)       DATA -> Block B1[0..3)
[3..1048583) HOLE -> zero
```

增长部分是 HOLE Extent。验证里 `arena_logical_bytes_delta=0.0`，说明没有为这段逻辑零字节分配全零 Block。

### 5.3 pwrite beyond EOF

用户在 offset=1 MiB+7 写入 4 bytes。

```text
旧布局：
[0..4) DATA -> Block B1[0..4)

新布局：
[0..4)          DATA -> Block B1[0..4)
[4..1048583)    HOLE -> zero
[1048583..1048587) DATA -> Block B2[0..4)
```

中间空洞仍是 HOLE，只有用户真正写入的 4 bytes 形成新 Block。验证里 `arena_logical_bytes_delta=4.0`，正是这个效果。

### 5.4 open with O_TRUNC

`O_TRUNC` 是 `open(2)` 的一部分，不是打开后后台再截断。DMS 在创建本地 open handle 前先完成 truncate 到 0 的原子发布；如果 Meta commit 失败，open 失败，不会留下“句柄打开成功但文件仍是旧版本”的中间状态。

### 5.5 O_APPEND

`O_APPEND` 不信任调用者传入的 offset。每次写入前，Filesystem 都重新读取当前授权 inode 的 EOF；如果并发写先提交，Meta 的 expected inode/object version 检查会返回 conflict，Filesystem 失效本地 binding 后重新解析 EOF 并重试。当前实现使用时间预算加最小重试次数，验证覆盖 16 个并发 writer。

## 6. 读路径如何处理 hole

读路径支持两类段：

- DATA 段：从本地 Block 读；本地没有时按 exact plan 从 Peer 拉取，再安装到本地 Node。
- HOLE 段：不访问 Peer，不申请 Region，不做 payload download，直接在输出中填零。

普通 WorkerService 读和进程内 DataCore 读都理解 zero segment。这样 Filesystem 不是唯一能读 sparse 文件的路径；只要 Meta 返回的布局包含 HOLE，底层读计划就可以一致处理。

```sequence
participant F as Filesystem
participant N as NodeState
participant P as Peer Node
F ->> N: read exact version range
N ->> N: 遍历 VersionLayout
N ->> N: HOLE 段写入零
N ->> P: DATA 段本地缺失时 PullBlock
P -->> N: block bytes
N ->> N: 按 logical offset 拼接 DATA 与 zero
N -->> F: 用户请求范围内的 bytes
```

## 7. Meta 的原子发布和幂等

文件内容提交使用 `FilesystemCommitVersionRequest`。请求同时携带：

- `operation_id` 与 `operation_digest`：同一次逻辑写的幂等身份。
- `inode` 与 `expected_inode_revision`：保护 inode size/attrs 不被并发写覆盖。
- `expected_object_version`：保护对象内容版本不被并发写覆盖。
- `candidate`：Node 准备好的 VersionCandidate，包含 DATA/HOLE Extent。
- `replica_proofs` 与 `new_replicas`：证明 DATA Extent 的 Block 可达；HOLE 不需要 proof。
- `new_size` 与 `mtime_unix_nanos`：本次要发布的文件属性。

Meta 在一个 actor turn 内完成校验、journal append、apply 和响应缓存。提交前，Meta 会重新验证候选布局：逻辑范围必须连续且完整覆盖 `logical_length`，HOLE 不得携带 Block，DATA 必须携带 Block/digest 且有完整副本证明，布局摘要必须与规范编码一致。Node 不能靠构造畸形 Extent 绕过文件尺寸与副本约束。

相同 `operation_id + digest` 再次到达时，Meta 直接返回第一次提交的原始 `FilesystemCommitVersionResponse`；即使之后同一 inode 又被其他操作更新，也不会错把当前版本当成旧 operation 的结果。

幂等结果不是永久保存。普通对象提交、Filesystem 内容版本提交和 Namespace 变更都按同一个 `operation_result_retention_records` 提交序号窗口回收；仍在等待失效 ACK/lease 收敛的文件版本操作或 Namespace 操作不能提前删除。这样既保留“未知结果重试返回原结果”的窗口，也避免 Meta 内存和 checkpoint 随操作数无限增长。

## 8. 失效、缓存与恢复边界

本轮继续沿用已有缓存一致性边界：

- 写入方在 commit 成功后用 Meta 返回的新 binding 回填本地 BindingCache。
- 其他 Node 通过 Watch 收到 `InvalidateFilesystemBinding`，先删除 inode binding cache，再 ACK。
- commit 需要等待相关可见性屏障后才向写入方返回；断流由 session lease 兜底。
- Meta WAL/checkpoint replay 会把 filesystem version operation 的精确响应和待完成的 visibility cursor 作为同一个生命周期恢复；重启后重试既不会拿到错误结果，也不会跳过尚未完成的失效屏障。ACK/lease 收敛后两份状态一起清除，超过显式保留窗口后再由统一 retention 回收。

当前仍是 write-through；M1.3 当时尚未实现的 unlink-open 回收、hard link、symlink 已由
文件身份阶段补齐，权限、xattr/ACL 与集群 statfs 已由 M1.4 补齐。write-back、dirty
page、文件锁和多 Meta 高可用仍未实现。

## 9. 代码阅读顺序

| 顺序 | 代码 | 重点 |
| :--- | :--- | :--- |
| 1 | [node_meta.proto](../protocol/proto/dms/v1/node_meta.proto) | `ExtentKind` 明确 DATA/HOLE；HOLE 不是空 block_id 的隐式哨兵 |
| 2 | [types.proto](../protocol/proto/dms/v1/types.proto) | `PayloadTarget.zero` 让普通读路径可以表达零段 |
| 3 | [version_layout.rs](../server/src/node/version_layout.rs) | `overlay_file_write`、`truncate_file`、`validate` 统一计算文件 Extent 布局 |
| 4 | [shared.rs](../server/src/node/filesystem/shared.rs) | `open` 处理 `O_TRUNC`；`write` 处理 `O_APPEND` 与 CAS 重试；`truncate` 走同一 commit |
| 5 | [data_core.rs](../server/src/node/data_core.rs) | `prepare_range`、`prepare_sparse`、`prepare_truncate` 是 Filesystem 到 Node 的窄入口 |
| 6 | [runtime.rs](../server/src/node/runtime.rs) | Node actor 内准备候选版本、materialize DATA/HOLE、finish prepared blocks |
| 7 | [worker_service.rs](../server/src/node/worker_service.rs) | WorkerService 把 `ReadTarget::Zero` 编码成协议 zero target |
| 8 | [peer_service.rs](../server/src/node/peer_service.rs) | Peer 读测试路径也识别 zero target，不把 hole 当远端 block 拉取 |
| 9 | [filesystem_meta.proto](../protocol/proto/dms/v1/filesystem_meta.proto) | `CommitFilesystemVersion` 是文件内容发布的唯一 Meta 服务合同 |
| 10 | [model.rs](../server/src/filesystem/model.rs) | `CommitFileVersionRequest` 表达 inode/object 双 CAS 与一次发布 |
| 11 | [meta_client.rs](../server/src/node/filesystem/meta_client.rs) | Node filesystem 领域请求到 Meta gRPC 请求的转换 |
| 12 | [service.rs](../server/src/meta/filesystem/service.rs) | Meta gRPC handler 只做边界转换和错误映射 |
| 13 | [meta runtime.rs](../server/src/meta/runtime.rs) | `filesystem_commit_version` 权威校验/原子发布；精确幂等响应与 visibility cursor 同生命周期；对象/文件/namespace 幂等结果统一有界回收；Watch invalidation |
| 14 | [metadata_journal.rs](../server/src/meta/metadata_journal.rs) | filesystem version commit journal record 包含对象版本和 inode binding；快照显式保存待完成的文件版本 visibility |
| 15 | [local_wal_journal.rs](../server/src/meta/local_wal_journal.rs) | WAL/快照编码保持 filesystem 字段和 visibility 生命周期可恢复，并兼容旧快照 |
| 16 | [read_cache_tests.rs](../server/src/node/read_cache_tests.rs) | 单元级回归覆盖 sparse、truncate、O_TRUNC、O_APPEND 与幂等重试 |

## 10. 验证证据

| 验证 | 结果 | 证据 |
| :--- | :--- | :--- |
| 单 VM 真实 E2E | PASS | [result.txt](../evidence/2026-09-16-filesystem-size-semantics/final-reviewed-single-vm/result.txt) |
| 单 VM 机器评价 | PASS | [evaluation.json](../evidence/2026-09-16-filesystem-size-semantics/final-reviewed-single-vm/evaluation.json) |
| 三 VM 真实 E2E | PASS | [result.txt](../evidence/2026-09-16-filesystem-size-semantics/final-reviewed-three-vm/result.txt) |
| 三 VM 机器评价 | PASS | [evaluation.json](../evidence/2026-09-16-filesystem-size-semantics/final-reviewed-three-vm/evaluation.json) |
| Linux 全工作区测试 | PASS | `dms-server` 359 passed、1 ignored；其余 workspace tests 全通过 |
| Linux 严格静态门禁 | PASS | `cargo fmt --check`、Clippy `-D warnings`、`git diff --check` |
| Python 验证工具 | PASS | evaluator/workload 11 tests |
| Workload 脚本 | 已落源码 | [filesystem_size_semantics_workload.py](../scripts/validation/filesystem_size_semantics_workload.py) |
| 独立评价器 | 已落源码 | [evaluate_filesystem_size_semantics.py](../scripts/validation/evaluate_filesystem_size_semantics.py) |
| 白盒合同 | 已落源码 | [filesystem-size-semantics-contract.json](../benchmarks/whitebox/filesystem-size-semantics-contract.json) |

最终评价器覆盖的行项目：

| 操作 | 期望 | 最终单 VM | 最终三 VM |
| :--- | :--- | :--- | :--- |
| truncate shrink | 远端 size=4 | PASS | PASS |
| truncate grow sparse | 远端 size=1048583，Arena 增量 0 | PASS | PASS |
| pwrite beyond EOF sparse | 远端 size=1048587，Arena 增量 4 | PASS | PASS |
| open O_TRUNC | 远端 size=0 | PASS | PASS |
| concurrent O_APPEND | 最终 size=192 | PASS | PASS |
| cross-node visibility | 远端 size=19 | PASS | PASS |

## 11. 当前边界

- 本轮只完成文件尺寸语义，不声明完整 POSIX 已完成。
- 当前没有 write-back；所有成功写入都是 write-through。
- HOLE 只表达逻辑零段，不是可靠持久存储能力。
- 单 Meta + 本地 WAL/checkpoint 不等于多 Meta 高可用。
- 本轮没有修改公开 KV SDK API。
