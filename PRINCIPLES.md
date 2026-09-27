# AFS 架构原则

状态：Accepted。本文定义 AFS 的长期产品与架构合同。代码已经具备的能力以[当前状态](docs/current-status.md)为准。

## 1. 通用 POSIX 是外部兼容合同

AFS 为常见 Linux 应用提供统一文件 Namespace 和 POSIX 接口，使应用无需接入专用对象 API。具体支持范围通过逐项兼容矩阵声明；未实现的 `mmap`、锁、xattr、ACL、`O_DIRECT`、目录同步等能力必须明确返回不支持，不以静默降级冒充正确实现。

## 2. Distributed BlobFs 是通用分布式主干

BlobFs 负责通用文件的数据布局、chunk/extent、复制、读取、修复、再平衡、容量管理和外部存储分层。它同时支持可变文件与不可变发布版本。`Blob` 表示分布式数据单元，不表示整个后端只读。

## 3. OwnerFs 是 1～4 节点 Workspace 特化路径

OwnerFs 面向一体机式 Agent Workspace。一个 Workspace 由一个 Home 节点持有，Home 使用本地普通文件系统；计算与 Home 共置时走本地路径，计算迁移后由远端 Node 通过 P2P 访问 Home。OwnerFs 不承担通用分布式 chunk、多副本写和跨大量节点聚合带宽。

## 4. Mutable 与 Published Immutable 是同一主干的两个 Profile

两种 Profile 共用 Namespace、inode、Meta 事务、Storage Service、文件布局、placement、transport、认证、配额、观测和生命周期账本。

Mutable Profile 负责 overwrite、append、truncate、写入排序、chunk version、`fsync`、cache coherence 和并发写语义。Published Immutable Profile 负责稳定切点、manifest、digest、发布门禁、P2P seed、去重、pin、淘汰和 spill。两种 Profile 使用不同的写入状态机，不建立两套文件系统。

## 5. Snapshot 由运行时显式触发

`close`、`flush` 和 `fsync` 不表示业务发布。运行时通过显式 Snapshot 接口取得目录树稳定切点。稳定视图独立后，活动文件继续通过新 generation 或 COW 写入。版本只有在 manifest、数据校验和可靠性策略全部满足后才可发布。

## 6. 本地磁盘构成近计算存储层

计算节点可以贡献 SSD、NVMe、HDD 或其他本地磁盘。系统根据介质能力、容量和故障域执行 placement。相对于集群外 OBS/S3，集群本地磁盘可以整体视为近计算缓存层；集群内部必须区分不完整副本、已校验缓存、持久副本和外部已提交副本。

## 7. 对象存储是可选的外部层

AFS 可以只使用集群本地磁盘形成可靠存储池。对象存储用于容量 spill、冷数据、归档和灾难恢复。数据只有在外部写入、校验和元数据提交全部完成后，才能以外部副本为依据逐出本地持久副本。活动可变数据不直接依赖对象存储的局部覆盖能力；首选将冻结 generation spill 到外部层。

## 8. Meta 管理权威，数据路径绕过 Meta

Meta 管理 Namespace、inode、文件布局、Workspace Home、版本、位置、租户和生命周期。客户端或 Node 取得布局后直接访问 Storage Service 或 OwnerFs Home。Meta 不转发文件内容，也不参与每个已解析数据块的读取。

## 9. 多源读取以明确的数据身份为前提

可变数据从受一致性协议管理的权威副本中选择读取目标。不可变发布版本通过 VersionId、manifest 和 digest 从持久副本、已校验缓存或 P2P seed 读取。未完成或未校验副本不可读取、不可计入可靠性、不可成为 seed。

## 10. 高性能接口复用同一文件语义

FUSE 提供低接入成本的 POSIX 路径。Native SDK 提供共享内存、注册 buffer、批量 range I/O、异步提交和 completion。块设备适配器服务 MicroVM。三种入口使用同一 Namespace、文件身份、布局和 Storage Service，不建立独立数据事实源。

## 11. 可靠性、兼容性和性能分别验收

可靠性验收覆盖副本、节点故障、Meta 故障、发布中断、spill、修复和逐出。兼容性验收覆盖 POSIX 操作、错误码、并发和持久化边界。性能验收按 OwnerFs 本地工作区、通用可变文件、大文件范围 I/O、镜像冷启动和多沙箱并发分别报告，不将单一场景结果外推到所有负载。

## 12. 事实、设计与研究保持可见边界

仓库文档使用 `Implemented`、`Experimental`、`Accepted Design`、`Draft`、`Research`、`Planned` 和 `Superseded` 标记状态。设计被接受不表示代码已经实现；实验成功不表示产品路径已经接入。
