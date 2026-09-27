# AFS 产品定位

状态：Accepted Design
实现状态：Partial
现行原则：[PRINCIPLES.md](../PRINCIPLES.md)

## 定位

AFS 是面向 Agent、Sandbox 和近计算工作负载的通用分布式文件系统。系统部署在业务计算集群内部，管理计算节点贡献的本地磁盘，并通过数据亲和、节点间直连和分布式数据布局降低远程存储路径成本。

AFS 对普通应用提供 POSIX 文件接口，对高性能应用提供 Native Async SDK，对 MicroVM 提供文件树或块设备适配。所有入口使用同一 Namespace、文件身份、布局和数据事实源。

## 核心价值

### 通用文件语义

应用以目录、文件、链接、权限和文件描述符组织数据。常见 Linux 应用无需改写为对象 API。兼容范围通过明确矩阵管理，不宣称所有负载都具有相同性能。

### 近计算数据路径

Node 与计算节点共置。本机存在数据时优先本地访问；缺失数据通过节点间链路获取。调度器可以查询 Workspace Home、持久副本和已校验缓存位置，将计算放置到数据附近。

### 计算节点本地磁盘池

本地 SSD、NVMe、HDD 等介质组成集群存储资源。BlobFs 在多个节点之间放置、复制、修复和再平衡文件数据。外部 OBS/S3 是可选的容量 spill、冷数据和归档层。

### 镜像与 Snapshot 优化

镜像、Snapshot 和 Checkpoint 具有显式稳定切点和固定版本，适合 manifest、digest、多源 P2P、去重、预取和分级缓存。消费者取得并校验 piece 后可以成为新 seed，降低大规模沙箱并发启动时的单源压力。

## 产品结构

### Distributed BlobFs

BlobFs 是通用分布式主干，支持两种数据 Profile：

- **Mutable Profile**：普通多读多写文件，支持 overwrite、append、truncate、并发写、`fsync` 和副本一致性。
- **Published Immutable Profile**：显式 Snapshot 产生的固定版本，支持 manifest、digest、P2P seed、去重、pin、spill 和淘汰。

两种 Profile 共用 Namespace、Meta、Storage Service、chunk/extent、placement、transport、认证、配额、观测和生命周期账本。

### OwnerFs

OwnerFs 服务 1～4 节点一体机式 Agent Workspace。一个 Workspace 由一个 Home 节点持有，数据保存在 Home 的本地普通文件系统中。本机访问走最短路径，远端访问回到 Home。OwnerFs 通过显式 Snapshot 或 Promote 将稳定视图交给 BlobFs，不隐式转换普通共享访问。

## 部署模式

### 集群自持久化

本地磁盘池保存满足故障域要求的持久副本。系统不依赖外部对象存储即可运行。缓存和副本通过内部状态明确区分。

### 外部层增强

本地磁盘承担热数据和计算邻近访问，OBS/S3 保存冷版本、容量溢出或归档副本。只有 `ExternalCommitted` 数据可以作为逐出本地持久副本的依据。

### 小集群 Workspace

OwnerFs 提供 Home 本地亲和路径。该模式优先降低 1～4 节点 Agent Workspace 的操作成本，不承担通用 BlobFs 的跨大量节点聚合吞吐。

## 重点工作负载

| 工作负载 | 入口 | 数据 Profile | 主要优化 |
| --- | --- | --- | --- |
| Agent Workspace | POSIX/FUSE | OwnerFs 或 Mutable | Home 亲和、小文件和元数据操作 |
| 通用共享文件 | POSIX/FUSE | Mutable | 分布式 chunk、副本、故障恢复 |
| 大文件与训练数据 | Native SDK | Mutable 或 Immutable | 批量 range、多节点并行、低复制开销 |
| OCI 镜像和文件树 | POSIX/文件树适配 | Immutable | 按需读取、digest、P2P、缓存 |
| MicroVM 根磁盘 | Block Adapter | Immutable + 私有写层 | range 读取、本地落盘或懒加载 |
| Snapshot/Checkpoint | Runtime API | Immutable | 稳定切点、发布、pin、spill |

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
4. 通用可变文件使用真正分布式的数据引擎；
5. 不可变发布版本增加滚雪球式 P2P；
6. OwnerFs 为 1～4 节点 Workspace 提供本地亲和特化；
7. 外部对象存储保持可选。
