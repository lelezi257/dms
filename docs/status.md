# 实现状态

更新时间：2026-09-29。

AFS 是面向业务集群近计算场景的通用 POSIX 分布式文件系统。产品包含两条后端：`DistributedFs`（简称 `DFS`）是通用分布式主线，`OwnerFs` 是 1～4 节点一体机 Agent workspace 的专用优化。

[README 架构图](../README.md#架构)展示已接受的目标部署和模块关系，不作为已实现能力清单；本页以下表格仍是实现状态的依据。

DFS 使用统一的数据模型承载普通可变文件和镜像、Snapshot 等固定版本工作负载：文件可变性由 inode 指向哪个 `FileVersion` 表达；已提交的 `FileVersion`、`LayoutRoot`、`ExtentMap` 和 `ChunkObject` 都不可变。固定版本读取、多源 P2P、缓存与 spill 直接复用这套事实源，不建立第二套 Blob 数据模型。

## 架构设计专题

[架构设计专题](architecture/design-topics.md)维护 DFS 的六个设计专题及其依赖关系。

专题一已经接受：[FileVersion、Extent 与 Chunk 数据模型](architecture/01-file-version-chunk-model.md)和 [RFC-0002](rfcs/0002-file-version-chunk-model.md)定义统一不可变版本模型、FileVersion EOF、隐式 Hole、单副本与多副本分叉点、端到端 Case 和 RPC 预算。专题二及 [RFC-0003](rfcs/0003-write-visibility-durability.md)固定用户/Node/Meta 时间线：普通 write、append 和 truncate 由 inode owner 串行归并到共享 dirty view 与 logical length，CommitTrigger 才产生 Chunk 和 FileVersion；`fdatasync/fsync`、flush/release、后台 writeback 和目录同步具有独立合同。专题三及 [RFC-0004](rfcs/0004-replication-state-machine.md)固定文件系统级不可变 ReplicationConfig、R1 Local Fast Path、RN Replication Path、Meta Placement 权威、Node ReplicationPlan、ReplicaAck/ChunkReceipt 与异步补副本合同。专题四及 [RFC-0005](rfcs/0005-local-chunk-engine-cow.md)固定 Layout COW 与 Physical COW 的边界、Truncate COW、LocalChunkRecord、可恢复 finalize、base Chunk 继承、per-chunk file 到 Pack 的演进以及 orphan reconciliation 合同。[专题六](architecture/06-reliability-performance-path.md)及 [RFC-0006](rfcs/0006-chunk-transfer-cache-spill.md)固定 DfsReadEngine、Copy role/state、SeedLease、合批与 attempt fencing、Inline/SHM/RDMA/Stream、VerifiedCache 和 Spill 合同。原专题五已经归并到专题一、二、四，不增加 FileMutation、AppendReservation、LengthHint、Seal 或独立 Snapshot/Image 数据类型。

接入模型已经实现为两个独立 mount：OwnerFs 与 DFS 分别建立 FuseSession、FUSE connection、inode/handle table 和缓存策略，只复用 `fuse` 模块代码与 `Backend` 接口。`DfsWriteSession` 是 DFS 专属类型；OwnerFs 不进入 FileVersion/Extent/Chunk 写入状态机。

Node RPC 使用四个职责文件：`control.rs` 负责 Node 间控制，`data.rs` 负责 Node 间入站数据服务，`meta.rs` 负责 Node 到 Meta 的调用，`peer.rs` 负责 Node 到其他 Node 的出站调用。OwnerFs 与未来 DFS Chunk RPC 先在对应职责文件内组织，全部专项完成后再评估物理拆分。

## 当前能力

| 能力 | 状态 | 已验证边界 |
| --- | --- | --- |
| CLI、TOML、日志、metrics、trace、错误框架 | 已实现 | Linux 构建与现有测试通过 |
| FUSE、SDK、REST 入口 | 已实现基础框架 | OwnerFs 路径已投入三 VM 验收 |
| MetaStore | 已实现单活动基础 | etcd、local-file、memory；后端 ACK 后发布可见状态 |
| OwnerFs | 已实现阶段能力 | 本机普通文件、跨 Node P2P、授权校验、句柄回收 |
| DistributedFs | Experimental R=1 | 独立 mount；inode 级 dirty view；write/flush/fdatasync/fsync/release 分离；`setattr(size)`、`truncate/ftruncate`、稀疏写与隐式 Hole 已接入；同步 dirty data 生成不可变 Chunk/FileVersion |
| FileVersion 数据模型 | Experimental | CommitPlanner 生成分块 Layout COW；Meta 以一次事务提交新 Chunk/Copy/Placement/LayoutRoot/FileVersion 与 inode head CAS，并验证 expected-base Extent 继承 |
| 写入可见性与同步合同 | Experimental R=1 | WriteLease、inode 级 InodeWriteState、DirtyExtentMap、DataOnly/Full commit 已接入；跨节点 owner routing 尚未实现 |
| Chunk 副本状态机 | Framework Implemented / R1 Experimental | ReplicationConfig 初始化、设备注册、PlacementSnapshot 缓存、R1 快路径、RN engine、ACK/receipt、Meta Copy/Placement/Task 原子提交和 gRPC/RDMA RPC 骨架已接入；RN 远端搬运与后台 worker 尚未实现 |
| Local ChunkEngine | Framework Implemented / R1 Experimental | BLAKE3 身份、per-chunk file、批量 durable finalize、no-replace publish、LocalChunkRecord/LocalCatalog、启动恢复与 reader FD pin 已接入；Pack、relocation、GC/reconciliation 尚未实现 |
| 固定版本多源读取 | Framework Implemented / Local DurableReplica Experimental | DfsReadEngine、ChunkReadOp/ReadBatch、Meta GetChunkSources、Peer ReadRanges wire、Source cache 与本机 DurableReplica 读取已接入；远端流式读取、VerifiedCache、SeedLease 与 Spill 尚未完成 |
| 外部对象存储 spill | 未实现 | 属于容量层选项，不是系统成立条件 |
| Native SDK 高性能数据面 | 基础框架 | SHM/RDMA 文件内容路径尚未接通 |

DFS length/truncate/sparse 基础路径在 Linux `dms-dev` VM 通过全 workspace 测试和严格 Clippy；真实 FUSE R1 E2E 已验证 `ftruncate` shrink→grow 不恢复旧尾部、路径 `truncate`、远偏移稀疏写、Hole 零读取，以及 Hole 不物化为零 Chunk。专题六第一阶段在 Linux `dms-dev` VM 通过 workspace 全目标全 feature check、严格 Clippy 和定向测试，验证本机 DurableReplica 读取、Meta 固定版本选源、gRPC Peer Range Read 最小服务端/客户端路径及失败换源。进程 crash、VM 掉电、Peer 授权攻击面和并发 sync 故障矩阵仍未验证，因此状态保持 Experimental R=1。

## OwnerFs 证据

最终 Linux Release 二进制在三 VM 功能验收中通过 15/15。带 `fh` 的属性操作与同句柄 I/O 保序；Home 校验打开句柄的实际根、peer 和授权；异步 RELEASE 支持有界重试；远端 daemon 异常退出后，Home 在 Meta 会话失效后回收遗留句柄。

固定 200×4 KiB、8 worker 负载下，完整远端 W2 两份 12 轮结果为 MooseFS 的 **0.768/0.780**；W1 本机私有根为 **0.482/0.451**；顺序 W2 为 **1.106/1.080**。这些结果只证明该固定负载，不代表完整 POSIX 或所有工作负载均优于 MooseFS。详细证据见[OwnerFs 修复与复验](reviews/2026-09-27-ownerfs-p2p-hardening.md)。

## 已知边界

- Meta 单活动围栏与选主尚未完成，不能宣称生产 HA。
- OwnerFs 尚缺根删除、跨节点根列举及部分常用属性操作。
- DFS 当前使用 inline Extent 列表；尚未实现 Extent tree、布局 compaction policy 与 RN 远端副本搬运。
- 专题四的热路径框架已经接入；Pack/relocation、reader pin 计数与删除状态机、orphan reconciliation 和掉电故障矩阵仍未实现。
- Chunk 内容身份已切换为带算法标识的 BLAKE3；旧 16 字节 FNV 原型格式不作为兼容持久格式。
- DFS commit 已用 FrozenCommit 把磁盘 I/O 和 Meta RPC 移出 inode 写状态锁；同一 inode 当前只允许一个 in-flight commit，后续需要等待/合并策略。
- 严格普通写跨节点可见性合同已经固化；当前只接受本地 WriteLease owner，远端 owner routing 尚未实现。
- DFS 已分离 `write/flush/fdatasync/fsync/release`，接通 `O_DSYNC/O_SYNC`，并增加定时后台 writeback、优雅退出 drain、inode sticky error 和 CommitBatch freeze；跨节点 owner routing、并发 sync 等待、故障恢复矩阵和 `fsync(dir)` 尚未实现。
- DFS `setattr(size)`、普通 `truncate/ftruncate`、`O_TRUNC`、写过 EOF、Shrink 后 Grow 和稀疏读取已经接通本地 owner 路径；远端 owner 转发、完整属性修改和并发 sync 故障矩阵尚未完成。
- CopyRecord 已拆为 `CopyRole + CopyState`，旧 JSON `Staging` 仅解码为不可读兼容态；Meta 选源当前只返回 Ready DurableReplica。VerifiedCache 与 ExternalCommitted 还没有真实晋升/回源状态机。
- DFS 固定版本读取已接入 `DfsReadEngine` 本机 DurableReplica 路径、Meta 选源和 gRPC Peer ReadRanges 最小路径；该路径只完成 Range 请求、服务端本机 Chunk 读取、frame 校验和候选失败换源。ReadGrant 完整校验、Seed Directory、ChunkCache、SpillStore、文件数据面 SHM/RDMA 和生产级连接/故障状态机尚未实现。
- RDMA 已有传输探测能力，文件内容仍走 gRPC P2P。
- 完整 POSIX、VM 掉电、长稳、容量压力和对象存储 spill 尚未验收。

产品合同见[产品定位](product-positioning.md)、[架构总览](architecture/overview.md)和[架构原则](../PRINCIPLES.md)。工程优先级见[Roadmap](../ROADMAP.md)与[实现任务](next.md)。
