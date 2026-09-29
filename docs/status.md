# 实现状态

更新时间：2026-09-29。

AFS 是面向业务集群近计算场景的通用 POSIX 分布式文件系统。产品包含两条后端：`DistributedFs`（简称 `DFS`）是通用分布式主线，`OwnerFs` 是 1～4 节点一体机 Agent workspace 的专用优化。

DFS 使用统一的数据模型承载普通可变文件和镜像、Snapshot 等固定版本工作负载：文件可变性由 inode 指向哪个 `FileVersion` 表达；已提交的 `FileVersion`、`LayoutRoot`、`ExtentMap` 和 `ChunkObject` 都不可变。固定版本读取、多源 P2P、缓存与 spill 直接复用这套事实源，不建立第二套 Blob 数据模型。

## 架构设计专题

[架构设计专题](architecture/design-topics.md)维护 DFS 的六个设计专题及其依赖关系。

专题一已经接受：[FileVersion、Extent 与 Chunk 数据模型](architecture/01-file-version-chunk-model.md)和 [RFC-0002](rfcs/0002-file-version-chunk-model.md)定义统一不可变版本模型、单副本与多副本分叉点、三个端到端 Case 和 RPC 预算。专题二及 [RFC-0003](rfcs/0003-write-visibility-durability.md)进一步固定用户/Node/Meta 时间线：普通 write 进入 inode owner 的共享 dirty view，CommitTrigger 才产生 Chunk 和 FileVersion；`fdatasync/fsync`、flush/release、后台 writeback 和目录同步具有独立合同。

接入模型已经实现为两个独立 mount：OwnerFs 与 DFS 分别建立 FuseSession、FUSE connection、inode/handle table 和缓存策略，只复用 `fuse` 模块代码与 `Backend` 接口。`DfsWriteSession` 是 DFS 专属类型；OwnerFs 不进入 FileVersion/Extent/Chunk 写入状态机。

Node RPC 使用四个职责文件：`control.rs` 负责 Node 间控制，`data.rs` 负责 Node 间入站数据服务，`meta.rs` 负责 Node 到 Meta 的调用，`peer.rs` 负责 Node 到其他 Node 的出站调用。OwnerFs 与未来 DFS Chunk RPC 先在对应职责文件内组织，全部专项完成后再评估物理拆分。

## 当前能力

| 能力 | 状态 | 已验证边界 |
| --- | --- | --- |
| CLI、TOML、日志、metrics、trace、错误框架 | 已实现 | Linux 构建与现有测试通过 |
| FUSE、SDK、REST 入口 | 已实现基础框架 | OwnerFs 路径已投入三 VM 验收 |
| MetaStore | 已实现单活动基础 | etcd、local-file、memory；后端 ACK 后发布可见状态 |
| OwnerFs | 已实现阶段能力 | 本机普通文件、跨 Node P2P、授权校验、句柄回收 |
| DistributedFs | Experimental R=1 | 独立 mount；inode 级 dirty view；write/flush/fdatasync/fsync/release 分离；同步 dirty data 生成不可变 Chunk/FileVersion |
| FileVersion 数据模型 | Experimental | Meta 以一次事务提交 Chunk/Copy/Placement/LayoutRoot/FileVersion 与 inode head CAS |
| 写入可见性与同步合同 | Experimental R=1 | WriteLease、inode 级 InodeWriteState、DirtyExtentMap、DataOnly/Full commit 已接入；跨节点 owner routing 尚未实现 |
| 固定版本多源读取 | Accepted Design | 身份和读取规则已确定，调度与数据路径尚未实现 |
| 外部对象存储 spill | 未实现 | 属于容量层选项，不是系统成立条件 |
| Native SDK 高性能数据面 | 基础框架 | SHM/RDMA 文件内容路径尚未接通 |

## OwnerFs 证据

最终 Linux Release 二进制在三 VM 功能验收中通过 15/15。带 `fh` 的属性操作与同句柄 I/O 保序；Home 校验打开句柄的实际根、peer 和授权；异步 RELEASE 支持有界重试；远端 daemon 异常退出后，Home 在 Meta 会话失效后回收遗留句柄。

固定 200×4 KiB、8 worker 负载下，完整远端 W2 两份 12 轮结果为 MooseFS 的 **0.768/0.780**；W1 本机私有根为 **0.482/0.451**；顺序 W2 为 **1.106/1.080**。这些结果只证明该固定负载，不代表完整 POSIX 或所有工作负载均优于 MooseFS。详细证据见[OwnerFs 修复与复验](reviews/2026-09-27-ownerfs-p2p-hardening.md)。

## 已知边界

- Meta 单活动围栏与选主尚未完成，不能宣称生产 HA。
- OwnerFs 尚缺根删除、跨节点根列举及部分常用属性操作。
- DFS 当前在 commit 时仍将文件版本物化为单个 Chunk 和 inline extent；已用 DirtyExtentMap 表达覆盖写，尚未实现分块 Extent 树、compaction 与 R=N 副本协议。
- 当前本机 Chunk digest 只用于首阶段损坏检测；在分布式去重与 P2P 前必须换成带算法版本的强摘要。
- DFS commit 已避免持有全局 handle table 锁执行磁盘 I/O 和 Meta RPC；当前仍持有 inode 级写状态锁完成首阶段 commit，后续需要用 freeze/snapshot 缩短 inode 临界区。
- 严格普通写跨节点可见性合同已经固化；当前只接受本地 WriteLease owner，远端 owner routing 尚未实现。
- DFS 已分离 `write/flush/fdatasync/fsync/release`，接通 `O_DSYNC/O_SYNC`，并增加定时后台 writeback、优雅退出 drain 与 inode sticky error；跨节点 owner routing、非阻塞 CommitBatch freeze、故障恢复矩阵和 `fsync(dir)` 尚未实现。
- RDMA 已有传输探测能力，文件内容仍走 gRPC P2P。
- 完整 POSIX、VM 掉电、长稳、容量压力和对象存储 spill 尚未验收。

产品合同见[产品定位](product-positioning.md)、[架构总览](architecture/overview.md)和[架构原则](../PRINCIPLES.md)。工程优先级见[Roadmap](../ROADMAP.md)与[实现任务](next.md)。
