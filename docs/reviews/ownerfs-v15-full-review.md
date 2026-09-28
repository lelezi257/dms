# OwnerFs 旧版迁移、FUSE/VFS 与性能全量审视（2026-09-27）

**结论：** 当前 OwnerFs 的本地优势成立：本轮最终代码在 Linux A/B/C 三 VM 上通过既有功能验收 15/15；W1 两份独立六轮分别为 MooseFS 的 **0.344/0.339 倍**。W2 仍与 MooseFS 总量基本持平；本轮白盒小优化后的独立六轮由 1.054 倍变为 **1.043 倍**，变化太小，不能认定为稳定性能收益。旧版最重要的**根创建故障恢复切点没有完整迁入**，因此 15/15 不能解释为“故障完备”。不启动 BlobFs/S6，不推送、合并或发布。

## RPC 与业务边界

**事实：** Node 并未把全部业务放在单个 `rpc.rs`。启动时将 [`OwnerFs::peer_executor()`](../../src/node.rs) 注入 `OwnerFilesHandler`；[`node/rpc/data.rs`](../../src/node/rpc/data.rs) 处理 mTLS 认证、Proto 转换、指标和 gRPC 错误，文件语义仍在 [`ownerfs.rs`](../../src/node/vfs/ownerfs.rs)。不需要为此恢复 actor 串行邮箱。

**本轮改动：** Meta 的 Reserve/Activate/Acquire/Validate/Recover 等事务状态机原先直接在 [`meta/rpc.rs`](../../src/meta/rpc.rs)。现在由新 [`meta/owner_roots.rs`](../../src/meta/owner_roots.rs) 的 `OwnerRootAuthority` 实现，启动时从 [`meta.rs`](../../src/meta.rs) 注入；RPC 只保留调用者身份绑定、字段校验、Proto/domain 转换和状态映射。`MetaService.RegisterNode/LookupNode` 仍在 RPC，作为后续小范围清理项。这次抽层不改变授权语义，也不修下表的故障窗口。

## 对照旧版仍缺什么

| 优先级 | 对照结论 | 代码证据与影响 |
| --- | --- | --- |
| P0 | 根创建的进程故障切点未迁完 | [`RootManager::create_root`](../../src/node/vfs/ownerfs/root.rs) 顺序为 Meta 预留→本地建目录和同步→catalog→Meta 激活；[`reconcile_on_startup`](../../src/node/vfs/ownerfs/root.rs) 仅扫描本机 catalog，`RecoverRoot` 仅接受已激活根。预留后/catalog 前崩溃可留悬空预留；catalog 后/激活前崩溃可使启动失败；激活提交而回复丢失时错误分支可能删掉 catalog。旧版 `source-agent-home-preview/server/homefs/src/home_fuse.rs:39-82` 对 pending 目录与中心记录做过对账，并有 `scripts/homefs/fault_cut_three_vm.py:100-170` 故障切点脚本。以上新版本结果是代码推断，尚未在新版本注入故障复现。 |
| P0 | 撤销预留缺少真正 CAS | [`StoreOwnerRootAuthority::abort_root`](../../src/meta/owner_roots.rs) 使用 `Missing("__never_exists_for_abort:...")` 占位条件，不校验刚读到的预留 revision；旧 Abort 与新 Reserve 竞争时有误删新预留的风险。需用预留身份/revision CAS 与并发测试闭合。 |
| P1 | `chmod/chown/atime/mtime` 没迁完 | 当前 [`LocalOwnerFs::setattr`](../../src/node/vfs/ownerfs.rs) 对这五类字段明确返回 `NODE_VFS_UNIMPLEMENTED`，只做 truncate；旧版 `source-agent-home-preview/server/homefs/src/p2p_rpc.rs:868-938` 已实现路径/句柄修改并有测试。Agent 工具链可能调用 `chmod`，现有 15 项验收未覆盖。 |
| P1 | 根删除、重建与全局列举没迁完 | 旧版中心与 Home 有根删除 prepare/物理删除/tombstone 流程；现版根级 `rmdir` 未实现。当前 `/ownerfs` `readdir` 只列本 Node 已知根，不主动发现别的 Node 新建根。需要持久目录清单和删除围栏；不能把普通子目录 `rmdir` 的 15/15 通过误记为根删除完成。 |
| P2 | 旧文件身份保证未完整迁移 | 旧版在 Linux 使用 `name_to_handle_at` 的不透明身份；现版以 `dev+ino+birthtime+type` 编码。即时同名重建 20/20 通过，但极端 inode 复用/文件系统不提供 birthtime 的长期边界尚未证明。 |

**已迁入且实测的能力：** 本机普通文件、Root 归属、P2P 远端文件命令、共享缓存收敛、旧 FD 同名重建、不同长度交替写、Meta/Home 进程重启后重新打开。旧 NFS 后端按新架构决策不迁入。`BlobFs` 仍是显式骨架，不纳入本轮 OwnerFs 结论。

## 从 RustFS 与 POSIX 项目借鉴什么

**事实：** [RustFS 官方架构](https://github.com/rustfs/rustfs/blob/2c5c43b0e0ed819366c6a18d0ebdb4bbfaa0b59d/ARCHITECTURE.md)描述的是 S3 对象 API→业务用例→存储引擎→I/O 核心；它不是 POSIX FUSE/VFS 的同类实现，不能复制一个不存在的 RustFS FUSE 层。更贴近当前接入的是 [JuiceFS 的 FUSE 适配](https://github.com/juicedata/juicefs/blob/main/pkg/fuse/fuse.go)与 [VFS 层](https://github.com/juicedata/juicefs/blob/main/pkg/vfs/vfs.go)：入口与文件语义分开。AFS 现有 FUSE→`Backend` trait→OwnerFs/BlobFs 分派已经符合此边界，没必要再加一层通用 VFS。

**可借鉴，但需按负载验证：** RustFS 的[有界并发准入](https://github.com/rustfs/rustfs/blob/2c5c43b0e0ed819366c6a18d0ebdb4bbfaa0b59d/rustfs/src/storage/concurrency/manager.rs)适合高并发尾延迟；[分级缓冲池](https://github.com/rustfs/rustfs/blob/2c5c43b0e0ed819366c6a18d0ebdb4bbfaa0b59d/crates/io-core/src/pool.rs)提示小读不要按最大 I/O 申请内存；不可变内容的 singleflight/cache 更适合未来 BlobFs。现版 OwnerFs 的 `state` Mutex 在本地 read/write 时覆盖文件 I/O，值得单独量化高并发阻塞后再缩短临界区。当前串行 W1/W2 数据不足以证明引入信号量或缓存池会更快，故本轮没有添加这些抽象或依赖。`Vfs::create_file` 里的旧 `probe_create` 仍是低优先级残留。

## 白盒优化与三 VM 复验

**改动：** 远端小文件只读 `open` 原先由 RPC Handler 完成本机 `open`、再通过 executor 进行第二次 `read` 预取；现在 [`LocalOwnerFs::peer_open`](../../src/node/vfs/ownerfs.rs) 在同一个已打开 OS 文件上直接读取 ≤4 KiB 内容并一同返回，RPC Handler 只封装应答。省去一次句柄表查找及重复文件身份校验，不改变共享缓存策略；读写打开、`O_PATH`/`O_DIRECT` 和短读仍按原路径。[`ownerfs_peer_contract.rs`](../../tests/ownerfs_peer_contract.rs) 在真实 mTLS OwnerFiles 调用中检验只读返回预取、读写打开不预取。

| 同场 Linux A/B/C，200×4 KiB，OwnerFs/MooseFS 交替六轮 | 改前 | 改后 |
| --- | ---: | ---: |
| W2 总量 p50，OwnerFs / MooseFS | 361.49 / 343.07 ms；**1.054×** | 350.86 / 336.25 ms；**1.043×** |
| B 首读 / 重读，OwnerFs p50 | 112.06 / 63.41 ms | 109.07 / 62.81 ms |
| A 端 `OwnerFiles.Open` 服务处理均值，3600 次 | 17.66 µs | 16.66 µs |
| B 端 `open` RPC 往返均值，3600 次 | 218.06 µs | 205.68 µs |

原始数据：[改前 W2](../../../experiments/results/2026-09-27-afs-ownerfs-full-review/w2-before.json)、[改后 W2](../../../experiments/results/2026-09-27-afs-ownerfs-full-review/w2-after.json)、[A/B 指标](../../../experiments/results/2026-09-27-afs-ownerfs-full-review/)。**推断：** 服务端 `Open` 的约 1 µs 缩短符合代码变化；整批 1% 左右的变化还混有 MooseFS 和环境波动，不能归因或宣称稳定提升。B 的跨 VM 请求往返比 A 端此处的本地处理贵一个数量级以上；下一轮性能工作应先在更大 W2 参数矩阵和并发负载定位，而非继续对这 1 µs 微调。

**功能与本地门槛：** 同一改后 release 二进制三 VM 功能验收 **15/15**，0 失败（[结果](../../../experiments/results/2026-09-27-afs-ownerfs-full-review/afs-fullreviewaccept.json)）；W1 四后端同场、两份独立各六轮 p50 OwnerFs/MooseFS **0.344/0.339**，满足 ≤0.80（[结果](../../../experiments/results/2026-09-27-afs-ownerfs-full-review/w1-after/result.json)）。W1 OwnerFs p50 233.93/226.72 ms，薄 FUSE 207.63/206.42 ms，Native 118.86/118.26 ms；Native 仅作参考。

Linux `cargo fmt --all --check`、`cargo test --workspace --all-features --exclude fuser`、严格 `cargo clippy --workspace --all-targets --all-features --exclude fuser -- -D warnings`、`cargo build --release --bins` 通过。vendored fuser 的自身测试目标按既有规则排除，生产 FUSE 由三 VM 验收覆盖。仍未跑根创建故障切点、VM 掉电、长稳、RDMA OwnerFiles 内容路径或完整 POSIX。下一入口应先修 P0 根创建/Abort CAS，再补 P1 的 `setattr` 与根生命周期；不因 15/15 掩盖这几个缺口。
