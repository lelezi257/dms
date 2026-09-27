# OwnerFs W2 远端路径优化（2026-09-26）

**结论（事实）：** 在同一 Linux A/B/C VM 与 200 个 4 KiB 文件的 W2 分段负载下，OwnerFs 从 v12 的 MooseFS **1.944 倍**降到正式 release 二进制复测的 **1.024 倍**，回到基本持平区间。它还没有证明稳定快于 MooseFS，也没有改变 W2 尚无数值验收门槛这一合同。此次只优化 OwnerFs；没有启动 BlobFs/S6，未 push、merge 或 release。

| 六轮会话 | OwnerFs p50/p95/p99 (ms) | MooseFS p50/p95/p99 (ms) | p50 比值 |
| --- | ---: | ---: | ---: |
| [v12 基线](../../../../experiments/results/2026-09-26-afs-ownerfs-w2/result-v11.json) | 642.5 / 759.6 / 759.6 | 330.5 / 387.4 / 387.4 | 1.944 |
| [分开缓存 TTL 后](../../../../experiments/results/2026-09-26-afs-ownerfs-w2/optimization/w2-entry-cache-v4.json) | 355.9 / 370.3 / 370.3 | 331.1 / 333.8 / 333.8 | 1.075 |
| [短同步操作调度优化 A](../../../../experiments/results/2026-09-26-afs-ownerfs-w2/optimization/w2-worker-v5-a.json) | 334.1 / 339.7 / 339.7 | 326.0 / 329.1 / 329.1 | 1.025 |
| [独立会话 B](../../../../experiments/results/2026-09-26-afs-ownerfs-w2/optimization/w2-worker-v5-b.json) | 337.4 / 365.8 / 365.8 | 326.4 / 359.8 / 359.8 | 1.034 |
| [最终 release 复测](../../../../experiments/results/2026-09-26-afs-ownerfs-w2/optimization/w2-final-v6.json) | 336.8 / 345.5 / 345.5 | 328.9 / 331.9 / 331.9 | **1.024** |

每行都是独立六轮、OwnerFs/MooseFS 交替顺序、相同操作语义的会话；200 个文件依次由 A 准备、B 初读与重读、B 覆盖写、A 回读。原始 JSON 含逐操作样本、挂载确认、二进制 SHA-256 和脚本参数。六轮的 p95/p99 都是最近秩最大样本，不能据此宣称稳定尾延迟。v12 与本次使用同类 W2 合同，但各会话不是严格同时的成对运行。

分段并非处处持平：最终复测中 B 初读、重读、覆盖写的阶段 p50 分别为 104.7/60.2/152.8 ms，MooseFS 为 82.4/37.5/146.8 ms；A 本机回读则是 18.2/61.8 ms。总量基本持平来自本机优势抵消远端读取劣势，不能解释为每种远端操作都已追平。

**原因与改动。** 白盒指标 `afs_ownerfiles_rpc_duration_seconds{side,method}` 显示 B 的重复路径查找带来大量短 RPC，Home 实际文件处理只占其中较小部分。此前 FUSE 把目录项与属性都设为零 TTL，内核每次走路径都重新向 Home `Lookup`。现在仅给 OwnerFs 远端**正目录项** 1 秒 TTL，属性仍为零；文件内容仍用 `DIRECT_IO`，重新打开仍经过 Home 的 `Open` 和身份校验。`fuser 0.16` 原 API 只接收一个 TTL，因此在 `third_party/fuser/` 加入分别编码 entry/attribute TTL 的小补丁，见其 [补丁说明](../../third_party/fuser/AFS-PATCH.md)。生产多线程 Tokio runtime 的短本地 Handler 改用 `block_in_place`，减少逐 RPC 投递阻塞线程池的调度成本；单线程测试 runtime 保留 `spawn_blocking`。这些改动直接对应 v4 与 v5 的两次下降，不引入新发布/Block/WAL 工作。

**正确性。** 目录项缓存会在 A 删除并立即同名重建后短暂保留 B 的旧 inode。Home 的不透明 `FileIdentity` 因此由 `dev+ino+类型` 加强为 `dev+ino+创建时间+类型`，避免 ext4 立即复用 inode 时把新文件当旧文件。B 的旧身份会被拒绝，随后重新 Lookup/Open；[最终 20 次即时重建重开](../../../../experiments/results/2026-09-26-afs-ownerfs-w2/optimization/immediate-reopen-v6.json) 20/20 得到新内容。[隔离三 VM 功能验收](../../../../experiments/results/2026-09-26-afs-ownerfs-w2/optimization/acceptance-v5.json) 15/15 通过，含 A/B 交替、旧 FD、重启后重新打开。底层本机文件系统须支持稳定的文件创建时间；不支持时目前返回错误，不退化到可能误认旧文件的身份。

**接口审视。** 未新增或修改 VFS `Backend`、MetaStore、Storage trait、核心目录层次或业务 Proto。新增的是 OwnerFiles 性能指标、FUSE 接入处的缓存 TTL 与 vendored `fuser` 小补丁，以及 Home Handler 的运行调度。`FileIdentity` 的 opaque 字节编码由 17 字节变为 29 字节；两端必须运行同版 Node，不能把它当跨版本持久标识。现有本地私有根页缓存及首次远端访问的失效边界仍是[前轮阶段复验](ownerfs-v12-stage-review.md)列出的待审项，不能因 W2 接近持平而宣称完整并发 POSIX 或 VM 故障已验收。

**Linux 验证。** `cargo fmt --all --check`、产品 workspace 全特性测试（`--exclude fuser`）、严格 Clippy 全特性全目标（同样排除 vendor 自身测试目标）、`cargo build --release --bins` 均通过。vendored 上游 `fuser` 自身有一个单测在此 Linux 环境异常终止，产品测试因此明确排除其测试目标；生产依赖的库仍参与编译和 FUSE 实机验收。留待下一阶段复核更广的 W2 文件数/大小与缓存并发时序；本次不为追逐 2% 差距继续改架构。
