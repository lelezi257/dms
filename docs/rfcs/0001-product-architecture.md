# RFC-0001：AFS 产品架构与数据 Profile

状态：Accepted
目标 Milestone：M0
权威原则：[PRINCIPLES.md](../../PRINCIPLES.md)

## 摘要

AFS 采用一条通用 Distributed BlobFs 主干、一条 OwnerFs 小集群特化路径，以及 Mutable 与 Published Immutable 两种 BlobFs 数据 Profile。集群本地磁盘构成近计算存储层，外部对象存储保持可选。

## 用户 Case

### 通用应用

应用通过 POSIX 路径创建、读写、改名和删除文件。文件数据分布在多个 Storage Node，节点故障后可以从其他权威副本读取和修复。

### 一体机 Agent Workspace

Agent 在 1～4 节点环境中访问 Workspace。数据保存在 Home 本地普通文件系统，调度器维持计算与 Home 亲和；远端访问回到 Home。

### 镜像与 Snapshot

运行时显式创建稳定切点。系统发布固定 Version，多个沙箱从持久副本和已校验 seed 读取。消费者可以在校验完成后成为新 seed。

### 容量溢出

本地磁盘达到水位后，稳定版本可以写入外部 OBS/S3。外部提交和校验完成后，系统按可靠性策略逐出部分本地副本，并在后续访问时 recall。

## 架构决定

### Distributed BlobFs

BlobFs 是通用文件数据引擎，负责 chunk/extent、replica group、读写、修复、再平衡、spill 和 recall。Mutable 和 Published Immutable 使用相同的 Namespace、Meta、Storage Service、数据容器和运维系统。

### Mutable Profile

Mutable Profile 使用稳定 FileId、Generation 和 ChunkIndex 标识数据，通过副本写入协议、chunk version 和 Meta 文件属性共同实现多读多写语义。

### Published Immutable Profile

Published Immutable Profile 通过显式 Snapshot 冻结 generation，生成 manifest 和 digest。发布后内容不可原地修改，可以从持久副本、已校验缓存和 P2P seed 读取。

### OwnerFs

OwnerFs 是 1～4 节点 Workspace 的独立 Backend。Home 使用本地普通文件，远端操作回 Home。OwnerFs 只独立物理数据路径、Home 路由和本地文件生命周期；认证、租户、管理入口、transport 和观测与 AFS 共用。

### 外部对象存储

对象存储不属于 AFS 的必需数据路径。集群本地持久副本可以独立形成可靠存储。外部层用于 spill、冷数据、归档和灾难恢复。

## 共用与分离

| 能力 | OwnerFs | BlobFs Mutable | BlobFs Immutable |
| --- | --- | --- | --- |
| POSIX Namespace | 共用入口 | 共用 | 只读版本视图 |
| 物理布局 | Home 普通文件 | 分布式 chunk/extent | 冻结 extent/manifest |
| 写入状态机 | Home 文件系统 | 副本版本协议 | Snapshot/Publish |
| 读取来源 | Home | 权威副本 | 副本、cache、seed、external |
| Meta | WorkspaceRoot/Home | inode/layout/version | Version/manifest/copy state |
| P2P | 远端回 Home | 权威副本选择 | 多源滚雪球分发 |
| Spill | Snapshot 后进入 BlobFs | 冻结 generation | 原生支持 |

## 不变量

1. Meta 不转发文件内容。
2. 未完成或未校验数据不可读取。
3. Cache 不自动计入持久副本。
4. `close` 和 `fsync` 不触发 Publish。
5. Snapshot 之后的写入不修改已冻结视图。
6. OwnerFs 与 BlobFs 之间不提供隐式跨后端 rename/link。
7. 对象存储不可用不影响只依赖本地持久副本的已提交读取。
8. 所有性能入口使用同一文件身份和数据事实源。

## 直接参考

3FS 提供通用可变文件的 chunk layout、replication chain、read-any、批量范围 I/O 和 Native SDK 参考。不可变 P2P Profile增加 manifest、digest、cache seed 和消费者扩散能力。OwnerFs 提供 AFS 自身的小集群本地亲和路径。

## 验收标准

- 新开发者可以从 README 识别三条数据路径及其状态。
- 当前代码能力和目标架构分别呈现。
- BlobFs Mutable 与 Immutable 的共用模块和独立状态机明确。
- OwnerFs 的 1～4 节点边界明确。
- copy state、发布门禁和 spill 门禁具有稳定术语。
- Roadmap 能将设计映射为纵向 E2E Milestone。
