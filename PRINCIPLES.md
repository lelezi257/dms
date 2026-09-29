# AFS 架构原则

状态：Accepted。本文定义 AFS 的长期产品与架构合同。代码已经具备的能力以[当前状态](docs/current-status.md)为准。

## 1. 通用 POSIX 是外部兼容合同

AFS 为常见 Linux 应用提供统一文件 Namespace 和 POSIX 接口，使应用无需接入专用对象 API。具体支持范围通过逐项兼容矩阵声明；未实现的 `mmap`、锁、xattr、ACL、`O_DIRECT`、目录同步等能力必须明确返回不支持，不以静默降级冒充正确实现。

## 2. DistributedFs 是通用分布式主干

DistributedFs（DFS）负责通用文件的数据布局、Chunk、复制、读取、修复、再平衡、容量管理和外部存储分层。DFS 支持普通多读多写文件；镜像、Snapshot 和 Checkpoint 是重点优化负载，不是独立后端。

## 3. OwnerFs 是 1～4 节点 Workspace 特化路径

OwnerFs 面向一体机式 Agent Workspace。一个 Workspace 由一个 Home 节点持有，Home 使用本地普通文件系统；计算与 Home 共置时走本地路径，计算迁移后由远端 Node 通过 P2P 访问 Home。OwnerFs 不承担通用分布式 Chunk、多副本写和跨大量节点聚合带宽。

OwnerFs 与 DistributedFs 使用两个独立 mount 和 FuseSession。两者复用 FUSE 模块代码与 Backend 接口，但不共享运行时 inode/handle table、notifier、缓存策略或数据模型。当前阶段不定义 OwnerFs 到 DFS 的数据转换合同。

## 4. 不可变 Chunk 是 DFS 的数据基座

提交完成的 `ChunkObject` 不原地修改。已提交文件内容的可变性由 `InodeRecord.head_version` 指向一系列不可变 `FileVersion` 表达；尚未提交的活动写入由 inode owner 的 `InodeWriteState/DirtyExtentMap` 管理。每个版本通过不可变 `LayoutRoot/ExtentMap` 复用已有 Chunk，并为覆盖范围引用新的 Chunk。小范围修改允许产生 Patch Chunk 和 Extent Overlay，后台 Compaction 控制碎片和空间放大。

## 5. FileVersion 是稳定读取与多源 P2P 的一致性边界

读取端先固定 `FileVersionId`，再解析 LayoutRoot、Extent 和 ChunkId。来自不同节点的数据只有在属于同一 FileVersion 且通过 Chunk 校验时才能组合。副本、已校验缓存和外部副本的位置变化不改变 FileVersion 或 Chunk 内容身份。

## 6. 同步操作提交文件版本，Snapshot 提交业务命名

普通 `write` 由 inode owner 排序并进入共享 `InodeWriteState`，无故障运行时对后续普通读取可见，但不提供故障恢复保证。`fdatasync` 必须提交文件数据及恢复数据所需的 Chunk、ExtentMap、LayoutRoot、FileVersion、length 和 `head_version`；`fsync` 在此基础上同步 `mtime/ctime` 等完整 inode 属性。`O_DSYNC/O_SYNC` 将相应同步屏障放到每次 write 返回之前。后台 writeback 可以提交内部 FileVersion，但不产生用户可依赖的同步完成点。文件同步不保证父目录项，目录项需要独立 `fsync(dir)`。

同步操作不自动创建业务 Snapshot、Alias 或保留策略。运行时通过显式 Snapshot/Pin/Publish 固定一个或多个 FileVersion；多文件一致视图使用 RootManifest。FUSE `flush` 和 `release` 只处理前端排空与 handle 生命周期，不作为持久化或业务发布边界。

## 7. 单副本和多副本只在 ChunkStore 以下分叉

文件布局层只调用 `ChunkStore::put` 并消费 `ChunkReceipt`。R=1 优先写本机磁盘；R=N 由 ReplicationEngine 在 ReplicaGroup 内流水复制。副本数量、Chain 顺序、Repair 和重配置不渗透到 FileVersion 与 Extent 层。

## 8. 本地磁盘构成近计算存储层

计算节点可以贡献 SSD、NVMe、HDD 或其他本地磁盘。系统根据介质能力、容量和故障域执行 placement。相对于集群外 OBS/S3，集群本地磁盘可以整体视为近计算缓存层；集群内部必须区分 Staged Copy、Verified Cache、Durable Replica 和 External Committed Copy。

## 9. 对象存储是可选的外部层

AFS 可以只使用集群本地磁盘形成可靠存储池。对象存储用于容量 Spill、冷数据、归档和灾难恢复。数据只有在外部写入、校验和元数据提交全部完成后，才能以外部副本为依据逐出本地持久副本。

## 10. Meta 管理权威，数据路径绕过 Meta

Meta 管理 Namespace、Dentry、InodeRecord、WriteLease、FileVersion、LayoutRoot、placement、租户和生命周期。客户端或 Node 取得 lease、路由与布局后直接访问 inode owner、ChunkStore 或 OwnerFs Home。Meta 不转发文件内容，也不参与每个已解析 Chunk 的读取或每个 FUSE WRITE。

## 11. 高性能接口复用同一文件语义

FUSE 提供低接入成本的 POSIX 路径。Native SDK 提供共享内存、注册 buffer、批量 range I/O、异步提交和 completion。块设备适配器服务 MicroVM。三种入口使用同一 Namespace、InodeRecord、FileVersion、LayoutRoot 和 ChunkStore，不建立独立数据事实源。

## 12. 可靠性、兼容性和性能分别验收

可靠性验收覆盖副本、节点故障、Meta 故障、版本提交中断、Spill、Repair 和逐出。兼容性验收覆盖 POSIX 操作、错误码、并发和持久化边界。性能验收按 OwnerFs 本地工作区、DFS 通用可变文件、大文件范围 I/O、镜像冷启动和多沙箱并发分别报告，不将单一场景结果外推到所有负载。

## 13. 事实、设计与研究保持可见边界

仓库文档使用 `Implemented`、`Experimental`、`Accepted Design`、`Draft`、`Research`、`Planned` 和 `Superseded` 标记状态。设计被接受不表示代码已经实现；实验成功不表示产品路径已经接入。
