# OwnerFs 全量实施前审视（2026-09-26）

> 本文是实施前审视快照。当前实现和复验结果见[OwnerFs v12 阶段复验](../reviews/ownerfs-v12-stage-review.md)；下文“尚未实现/待验收”均按审视当时状态阅读。

状态：**三处实施前合同已修复；业务实现与性能验收尚未通过**。本文件审视已确定的模块边界，不宣称业务已实现。下一步的目标是恢复旧 Home 分支的 P2P 功能与 W1 性能门槛；旧实测不是新实现的验收证据。

## 先固定业务事实与决策

- **决策**：一个 WorkspaceRoot 有一个 Home A，根内只有 A 的普通文件是可写事实源。A 的本地 FUSE 操作与 B 的远端 P2P 操作可同时进入 A，并在同一套文件/句柄语义下执行。B 获得自己的 RootGrant 不改变 A 的权限，不触发独享/共享模式切换，也不要求 A 为 B 加入发撤销 ACK。授权持有者可增减，Home 不因访问者变化而迁移。依据：[原则](../../PRINCIPLES.md)、[架构 B 段](../architecture.html)。
- **决策**：真正删除根、撤销授权、Home 进程恢复才推进围栏/授权代并处理在途请求。FUSE 缓存策略可以单独调整，不是授权模式。首版关闭 FUSE 内容缓存/writeback，属性及正负目录项重新验证；底层普通文件仍用 Linux page cache。依据：[架构缓存段](../architecture.html)。
- **事实**：`root.rs`/`meta.proto` 已移除 `RootMode` 与“B 加入前 A ACK”；Home 对 B 授权的首次权威验证已定义接口和缓存准入，但 Meta RPC/OwnerFiles 业务 Handler 仍返回 `UNIMPLEMENTED`。依据：`src/node/vfs/ownerfs/root.rs`、`common/protocol/proto/meta.proto`、`src/meta/rpc.rs`。

## 旧分支的可复验目标

| 目标 | 旧证据与新分支验收方式 |
| --- | --- |
| 三节点 P2P 功能 | **事实**：旧分支安装包 A/B/C 10/10 步通过：启动挂载、跨节点 close-to-open/目录操作、并发根创建唯一 Home、旧 FD 在 rename/unlink/同名文件重建后仍指旧文件、不同长度交替写、位置查询、Meta 重启、Home 进程重启后重新打开。新分支须在 Linux 重跑同语义脚本；NFS 已退出产品范围，不要求移植 NFS。旧脚本：`source-agent-home-preview/scripts/homefs/accept_three_vm.py`。 |
| W1 本地性能 | **事实**：旧合同是相同负载与默认 MooseFS 对照，**两份独立、每份六轮有效样本**，每份 DMS/MooseFS p50 ≤ 0.80。旧最佳可比场 0.375/0.381（DMS/薄 FUSE/Native/MooseFS 同场 234/191/114/625 ms 与 240/199/112/630 ms）；旧阶段候选 0.494/0.497。新分支须完整重测，不能沿用旧数值；Native 与薄 FUSE用于定位成本，不是 W1 硬门槛。旧证据：`source-agent-home-preview/docs/reviews/agent-home-preview-stage-review.md`。 |
| W2 远端性能 | **事实**：旧 P2P 六轮混合用例与 MooseFS 大致持平，最终同场比值约 0.978，另一次可比场 0.920；没有单独的 W2 数值验收门槛。新分支按同一操作、缓存和耐久条件报告总量及初读/重复读/写分段，明显退化须定位。 |
| 可靠性边界 | **事实**：旧版旧远端 FD 跨 Home 进程重启返回 `ESTALE`，重新打开可恢复；不承诺运行中 FD 无感续接、原 VM 永久丢盘后的接管、等物理耐久、完整 POSIX 或长稳。新分支不能把本机保存和 MooseFS 默认 ACK 当作同等耐久优势。 |

旧 W1 包含一级目录创建/删除、200 个小文件的创建/同步/重开/改写/重命名/删除；预建 workspace 的 225/206、227/209 ms DMS/薄 FUSE 是路径诊断，不能替代完整 W1。旧 0.375/0.381 依赖本地短属性/目录项缓存与远端首次访问时失效；新设计首先用保守缓存保证正确，再测性能，若不足 0.80 才将短缓存作为受一致性门禁的优化。

## 核心模块覆盖矩阵

| 链路/用例 | 已有正式边界 | 实施时扩展 | 新核心块判断 |
| --- | --- | --- | --- |
| POSIX 接入、namespace 与 inode/句柄 | `node/fuse.rs`、`node/vfs.rs` 的 `Backend`/`Vfs` | FUSE 所有目标回调、进程内 inode/handle 映射和 `forget`、两后端隔离；可在 `node/fuse/state.rs` 放接入层状态 | 无新接口类；`state.rs` 仅是已有 FUSE 的内聚实现文件 |
| 本地根创建与授权 | `ownerfs/root.rs` 的 `RootLifecycle`/`RootManager`/`RootMeta`，`meta.proto` OwnerRoots | reserve → 本机目录及记录持久准备 → activate；去掉 `RootMode`，Home 与 B 授权并存；RootId 用无损名字编码 | 无新业务层 |
| Meta 权威、重启与位置查询 | `meta.rs`、`meta/rpc.rs`、`meta/rest.rs` 已有进程/API；`meta/store.rs` 已定义持久权威接口 | 实现 etcd 条件事务、请求去重、revision/watch、Node 会话、恢复对账；统一驱动 gRPC/REST | 核心边界已落合同；真实后端尚未实现，不另加 Master/Actor 层 |
| 本地文件、身份、持久操作 | `ownerfs/files.rs`、`storage.rs` 的 `FileStore/FileHandle/DirectoryHandle` 与 `storage/localfs.rs` | 文件身份/FD 表、普通文件与目录操作、权限、symlink/link、fsyncdir；补现有 trait 的缺失方法 | 无新核心块；本地/远端身份均留在 `files.rs` |
| B 远端操作 | `ownerfs/remote.rs` 的 `RemoteFiles`，`node_data.proto` 的 OwnerFiles，`node/rpc/data.rs`，`node/rpc/peer.rs` 的传输 adapter | 扩展 `RemoteFiles` 的 mkdir/create/rename/setattr 等；在 `node/rpc/peer.rs` 转 Proto，内容复用 gRPC/RDMA 数据通道；A Handler 验证授权后调用同一 OwnerFs 文件执行器 | 无新业务层；`peer.rs` 是已确定的适配文件 |
| 本机进程恢复 | `ownerfs/catalog.rs` 的持久根记录/生命周期锁，`RootMeta::recover_root`，`meta.proto` RecoverRoot | 实现记录落盘/数据目录排他锁、Meta 条件恢复、新 Home 会话围栏、B 旧句柄 `ESTALE` | 无新核心块 |
| 缓存与 close-to-open | `node/fuse.rs`、`ownerfs/files.rs`、RootManager | 基线 direct-io、无 writeback、TTL=0；B 改写后 A 重开可见。若启短缓存，OwnerFs 文件身份映射与 FUSE invalidation 同步 | 无新业务层；性能优化不得改授权合同 |
| 根删除/同名根重建 | `RootLifecycle`/OwnerRoots 现仅创建、查找、恢复；现架构 W4 要求新世代 | 扩展现有 trait/Proto 的 delete/reconcile 状态、旧授权围栏、tombstone 与新根身份/epoch | 无新业务层，但**当前接口方法确有缺口**；旧候选版没有同名根重建，不可当成已恢复功能 |

## 已修复的三处合同与仍待实施的部分

1. **A 验证 B 授权（合同已修）**。OwnerRoots 已增加 `ValidateRootAccess`；P2P 出示字段不携带 rights，A 用认证连接得到的 peer 身份首次向 Meta 校验，按返回权利缓存同一 grant。B 加入不撤销 A。`RootManager` 的缓存准入已实现并有定向测试；Meta RPC、真实节点认证和 OwnerFiles Handler 尚未接线，首次远端访问的控制 RTT 也未实测。
2. **持久权威接口（合同已修）**。`meta/store.rs` 已定义条件事务与幂等结果同次提交、线性读、有序 revision/watch、Node 会话及恢复扫描，并规定撤权命令先持久记录、A 拒绝旧授权 ACK 后才提交授权代变化。首个 etcd 后端尚未实现，Meta 权威 RPC 继续返回 `UNIMPLEMENTED`。
3. **旧模式与控制故障语义（合同已修）**。已移除 `RootMode` 和 B 加入时推进围栏代/等待 A ACK 的条款；同一根的 A/B 授权共享有效围栏代。`on_watch_disconnected` 只标记控制会话无效并保守拒绝新准入，不伪造撤权 ACK；`revoke_root` 与 `invalidate_all` 分别处理单根撤权和本进程失效。控制面失联时“已有安全授权继续”的优化还需真实持久撤权状态机与重连对账证明，当前代码选择 fail-closed。
4. **性能与缓存（验收阻塞，非架构块）**。保守 FUSE 缓存是正确性起点，但旧 W1 的 0.375/0.381 含短缓存收益。先按完整 W1 对 Native/薄 FUSE/MooseFS 分段复测；若不达 ≤0.80，再用文件身份驱动的定向失效/保守 TTL 缩短局部成本，并重跑 A/B 交替写、不同长度、重建与旧 FD。不要把旧数字当作新实现已达成。

**额外范围边界（待实现，不冒充旧版已有能力）**：当前 `RootId(pub String)` 的例子直接用目录名 `job-42`；若允许删除根后复用同名，必须让根的不可变身份与名字分开，或至少把 epoch 纳入身份键，不能把新根和旧根的 inode/授权/句柄混在一起。现有 `RequestContext` 和本机/远端文件路径尚未完整证明 POSIX 凭据/补充组/权限恢复；旧阶段报告本就有 `chmod 000` 后恢复权限失败的已知问题。这两项应留在既有 root/files/VFS/Storage 类型中补齐，不能把旧 10/10 当作完整 POSIX 验收。

**结论（推断）**：三处已识别的核心合同缺口已补，现有 VFS/OwnerFs/Storage/Proto/transport 划分可进入业务实现，原则上不再新增核心块。合同通过不等于功能通过：etcd MetaStore、根生命周期、A/B 真实 P2P 文件操作、缓存屏障与恢复仍需按矩阵实现并用 Linux 三 VM 与 W1/W2 重验；不同时启动 BlobFs。
