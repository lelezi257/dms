# RFC-0001：AFS 产品架构

状态：Accepted（由 RFC-0002 补充数据模型）
目标 Milestone：M0
权威原则：[PRINCIPLES.md](../../PRINCIPLES.md)

## 摘要

AFS 采用一条通用 `DistributedFs`（DFS）主线和一条 `OwnerFs` 小集群特化路径。DFS 面向通用 POSIX 多读多写，并针对镜像、Snapshot、Checkpoint 等写后很少修改的固定版本工作负载优化。集群本地磁盘构成近计算存储层，外部对象存储保持可选。

## 用户 Case

### 通用应用

应用通过 POSIX 路径创建、读写、改名和删除文件。文件数据分布在多个 Storage Node，节点故障后可以从其他权威副本读取和修复。

### 一体机 Agent Workspace

Agent 在 1～4 节点环境中访问 Workspace。数据保存在 Home 本地普通文件系统，调度器维持计算与 Home 亲和；远端访问回到 Home。

### 镜像与 Snapshot

业务固定一个 `FileVersionId` 或一组版本根。多个沙箱从持久副本和已校验 seed 按需读取 Chunk；消费者校验完成后可以成为新 seed。后续文件修改生成新 Chunk 和新 FileVersion，不改变旧视图。

### 容量溢出

本地磁盘达到水位后，稳定 FileVersion 引用的 Chunk 可以写入外部 OBS/S3。外部提交和校验完成后，系统按可靠性策略逐出部分本地副本，并在后续访问时 recall。

## 架构决定

### DistributedFs

DFS 是通用文件数据引擎，负责 FileVersion、extent、ChunkObject、replica、读写、修复、再平衡、缓存、spill 和 recall。普通可变文件与固定版本工作负载共用 namespace、Meta、Node、ChunkStore、placement、transport 和运维系统。

### 统一不可变数据基座

DFS 的持久数据模型是：

```text
mutable InodeRecord.head_version
  → immutable FileVersion
  → immutable LayoutRoot / ExtentMap
  → Extent[]
  → immutable ChunkObject
```

文件修改通过提交新 Chunk、新布局和新 FileVersion 表达。系统不要求单独的 BlobRecord、BlobManifest 或 Blob API。完整合同见 [RFC-0002](0002-file-version-chunk-model.md)。

### 固定版本优化

镜像、Snapshot 和 Checkpoint 固定 `FileVersionId` 后即可安全进行 range read、校验缓存和多源 P2P。Alias、Pin/Retention 和 RootManifest 是引用与生命周期能力，不建立另一套数据实体。

### OwnerFs

OwnerFs 是 1～4 节点 Workspace 的独立 Backend。Home 使用本地普通文件，远端操作回 Home。OwnerFs 独立物理数据路径、Home 路由和本地文件生命周期；认证、租户、管理入口、transport 和观测与 AFS 共用。

### 外部对象存储

对象存储不属于 AFS 的必需数据路径。集群本地持久副本可以独立形成可靠存储。外部层用于 spill、冷数据、归档和灾难恢复。

## 共用与分离

| 能力 | OwnerFs | DFS 普通文件 | DFS 固定版本工作负载 |
| --- | --- | --- | --- |
| POSIX Namespace | 共用入口 | 通用读写 | 同一文件的固定版本视图 |
| 物理布局 | Home 普通文件 | FileVersion/Extent/Chunk | 同一模型 |
| 写入 | Home 文件系统 | 新 Chunk + 新 FileVersion | 不修改固定版本 |
| 读取来源 | Home | 权威副本 | 副本、verified cache、seed、external |
| Meta | WorkspaceRoot/Home | inode head、版本、布局 | FileVersion 引用、Pin、Alias、RootManifest |
| P2P | 远端回 Home | 权威副本选择 | 多源分发 |
| Spill | 显式迁移后进入 DFS | 稳定 Chunk 可选 | 原生适配 |

## 不变量

1. Meta 不转发文件内容。
2. 未 finalize 或未校验的数据不可读取。
3. Cache 不自动计入持久副本。
4. `fsync` 强制完整 FileVersion 持久化，但不自动创建业务 Alias、Pin 或 RootManifest。
5. 已提交的 FileVersion、布局和 Chunk 不原地修改。
6. 多源读取必须固定同一 FileVersion。
7. OwnerFs 与 DistributedFs 之间不提供隐式跨后端 rename/link。
8. 对象存储不可用不影响只依赖本地持久副本的已提交读取。
9. 所有性能入口使用同一文件身份和数据事实源。

## 直接参考

3FS 提供通用可变文件的 chunk layout、replication chain、read-any、批量范围 I/O 和 Native SDK 参考。AFS 在统一不可变 Chunk 基座上增加 FileVersion 固定视图、校验缓存和消费者扩散能力。OwnerFs 提供小集群本地亲和路径。

## 验收标准

- 新开发者可以从 README 识别 OwnerFs 和 DistributedFs 的定位及状态。
- 当前代码能力和目标架构分别呈现。
- 普通可变文件与固定版本工作负载共用一套数据模型。
- OwnerFs 的 1～4 节点边界明确。
- Roadmap 能将设计映射为纵向 E2E Milestone。
