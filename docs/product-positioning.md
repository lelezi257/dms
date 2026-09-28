# AFS 产品定位

状态：Accepted Design
实现状态：Partial
现行原则：[PRINCIPLES.md](../PRINCIPLES.md)

## 定位

AFS 是面向 Agent、Sandbox 和近计算工作负载的通用分布式文件系统。系统部署在业务计算集群内部，管理计算节点贡献的本地磁盘，并通过数据亲和、节点间直连和分布式数据布局降低远程存储路径成本。

AFS 对普通应用提供 POSIX 文件接口，对高性能应用提供 Native Async SDK，对 MicroVM 提供文件树或块设备适配。DFS 的 POSIX、SDK 和块设备入口使用同一 Namespace、文件身份、版本、布局和 Chunk 数据事实源；OwnerFs 保持独立的 Home 本地普通文件模型。

## 产品结构

```text
AFS Node
├── /mnt/dfs → FuseSession<DistributedFs>
│   ├── 通用 POSIX 多读多写
│   ├── Immutable FileVersion + LayoutRoot + ChunkObject
│   ├── R=1 / R=N、多源 P2P、Cache、Repair
│   └── 可选对象存储 Spill
└── /mnt/ownerfs → FuseSession<OwnerFs>
    └── 1～4 节点 Agent Workspace 的 Home 本地亲和路径
```

两个 mount 复用 FUSE 模块代码和 `Backend` 接口，但拥有独立的 FUSE connection、会话 inode/handle table、notifier 和缓存策略。

### DistributedFs

DFS 是通用分布式主干，负责 FileVersion、Extent、Chunk、ReplicaGroup、读取、写入、修复、再平衡、缓存和 Spill。文件对用户可变；已提交 FileVersion、LayoutRoot 和 ChunkObject 不可变。镜像、Snapshot、Checkpoint 和只读数据集通过版本 Pin、多源读取、消费者种子和缓存获得额外性能，不转换成另一种 Blob 对象。

### OwnerFs

OwnerFs 服务 1～4 节点一体机式 Agent Workspace。一个 Workspace 由一个 Home 节点持有，数据保存在 Home 的本地普通文件系统中。本机访问走最短路径，远端访问回到 Home。当前阶段不设计 OwnerFs 与 DFS 之间的数据转换合同。

## 核心价值

### 通用文件语义

应用以目录、文件、链接、权限和文件描述符组织数据。常见 Linux 应用无需改写为对象 API。兼容范围通过明确矩阵管理，不宣称所有负载都具有相同性能。

### 近计算数据路径

Node 与计算节点共置。本机存在数据时优先本地访问；缺失数据通过节点间链路获取。调度器可以查询 Workspace Home、持久副本和已校验缓存位置，将计算放置到数据附近。

### 计算节点本地磁盘池

本地 SSD、NVMe、HDD 等介质组成集群存储资源。DFS 在多个节点之间放置、复制、修复和再平衡 Chunk。外部 OBS/S3 是可选的容量 Spill、冷数据和归档层。

### 不可变工作负载优化

镜像、Snapshot 和 Checkpoint 具有明确的稳定版本，适合 Digest、多源 P2P、去重、预取和分级缓存。消费者取得并校验 Chunk 后可以成为新 Seed，降低大规模沙箱并发启动时的单源压力。

## 部署模式

### 集群自持久化

本地磁盘池保存满足故障域要求的持久副本。系统不依赖外部对象存储即可运行。缓存和持久副本通过状态明确区分。

### 外部层增强

本地磁盘承担热数据和计算邻近访问，OBS/S3 保存冷版本、容量溢出或归档副本。只有 `ExternalCommitted` 数据可以作为逐出本地持久副本的依据。

### 小集群 Workspace

OwnerFs 提供 Home 本地亲和路径。该模式优先降低 1～4 节点 Agent Workspace 的操作成本，不承担 DFS 的跨大量节点聚合吞吐。

## 重点工作负载

| 工作负载 | 入口 | 数据路径 | 主要优化 |
| --- | --- | --- | --- |
| Agent Workspace | POSIX/FUSE | OwnerFs 或 DFS | Home 亲和，或通用分布式一致性 |
| 通用共享文件 | POSIX/FUSE | DFS | Extent、Chunk、副本、故障恢复 |
| 大文件与训练数据 | Native SDK | DFS | 批量 Range、多节点并行、低复制开销 |
| OCI 镜像和文件树 | POSIX/文件树适配 | DFS 固定 FileVersion | 按需读取、Digest、P2P、缓存 |
| MicroVM 根磁盘 | Block Adapter | DFS 固定 FileVersion + 私有写层 | Range Read、本地落盘或懒加载 |
| Snapshot/Checkpoint | Runtime API | DFS Pin/RootManifest | 稳定切点、保留、P2P、Spill |

## 非目标

- 不保证所有 POSIX 负载都达到最优性能。
- 不把对象 API 作为应用访问文件的必经入口。
- 不把 Meta 放入文件内容转发路径。
- 不把未校验缓存计入持久副本。
- 不用 `close` 或 `fsync` 代替业务 Snapshot/Publish。
- 不把磁盘 Snapshot 自动解释为完整的 VM 内存和设备状态。

## 差异化

AFS 的差异化来自能力组合：

1. 通用 POSIX Namespace；
2. 计算节点本地磁盘成为一等存储资源；
3. 数据位置参与计算调度；
4. 通用可变文件使用真正分布式的 FileVersion/Extent/Chunk 引擎；
5. 固定 FileVersion 支持校验后的滚雪球式 P2P；
6. OwnerFs 为 1～4 节点 Workspace 提供本地亲和特化；
7. 外部对象存储保持可选。
