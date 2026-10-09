# AFS 架构原则

本页记录稳定原则；实现进度和验收状态见 [当前计划](docs/development/plan.md) 和 [状态摘要](docs/status.md)。

## POSIX 优先

AFS 通过普通目录、文件、读、写、sync 和 close 暴露共享文件命名空间。默认一致性目标是同一挂载内及时可见，跨挂载 close-to-open。高性能 SDK 只能作为 DFS 的附加入口，不能改变文件语义。

## DFS 是通用路径

`DistributedFs` 负责通用分布式文件系统能力：布局、chunk、副本、读计划、修复、缓存、容量管理和可选 spill。镜像、快照和 checkpoint 是稳定文件版本，不是独立 Blob 文件系统。

## OwnerFs 是小集群 workspace 路径

`OwnerFs` 面向 1 到 4 节点 Agent workspace。workspace 有 Home 节点，Home 用普通本地文件保存字节；远端 worker 通过 peer 回到 Home。OwnerFs 和 DFS 是不同挂载、不同状态机、不同缓存策略，不互相继承验收结论。

## 不可变 chunk 构成 DFS 基础

已提交 `ChunkObject` 不可变。文件仍然可变，因为 sync 前有 dirty view，sync 后 inode 的 `head_version` 可以从一个不可变 `FileVersion` 前进到另一个版本。

## FileVersion 是读一致性边界

已提交读计划必须固定 `FileVersion`、布局和长度。不同节点的数据可以组合，前提是它们属于同一个解析出的版本并通过 chunk 身份校验。副本位置、缓存位置和外部位置可以变化，但不能改变版本身份。

## Sync 提交文件状态，不创建业务发布

`fdatasync` 提交文件数据和恢复所需元数据；`fsync` 还包括完整 inode 属性。文件 sync 不等于父目录 sync，目录项需要 `fsync(dir)`。成功 close 会 flush 之前写入并提交可恢复状态；release 只清理资源。

## 复制在 ChunkStore 之下

文件布局代码消费 `ChunkReceipt`。单副本和多副本路径在 `ChunkStore::put_batch` 之下分叉，在 Meta 提交新版本前汇合。副本数是文件系统初始化策略，不写入每个文件版本或 extent。

## 本地盘是近计算存储池

计算节点可以贡献 SSD、NVMe、HDD 或其它本地盘。AFS 区分持久副本、验证缓存和外部已提交副本；验证缓存不能自动冒充持久副本。

## 对象存储是可选 spill

AFS 可以只依赖集群本地盘运行。外部对象存储只作为 spill、冷数据、归档或灾备层。只有完成外部写入、校验和 Meta 提交后，才能因为外部副本而淘汰本地数据。

## 第三方源码保持原样

开源依赖源码不应长期维护私有补丁。当前 `third_party/fuser` 仍是迁移前阻塞项，不能通过降低文件系统语义来换取表面解耦。
