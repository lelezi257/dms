# 架构总览

AFS 把用户可见的文件语义和文件字节搬运分开。应用通过 FUSE 挂载访问 OwnerFs 或 DFS；DFS 也可以在后续提供专用 SDK。`afs-node` 靠近工作负载，负责挂载、缓存、复制和节点间传输；`afs-meta` 负责命名空间、租约、文件版本、布局、放置和副本目录。

![AFS architecture](images/overview.svg)

## 基本原则

1. Meta 只掌握权威状态，不代理稳定数据流。命名空间、inode、版本、布局和副本记录由 Meta 提交；文件字节在节点本地盘、节点间传输、验证缓存和可选冷存之间移动。
2. DFS 把已提交数据表示为不可变 chunk。普通写入先进入 inode 的 dirty 状态；显式 sync、同步写标志、close-time flush 或后台策略触发提交，形成新的 `FileVersion`。
3. 读取先固定一致视图再选择数据来源。跨挂载可见性按 close-to-open；同挂载读还可看到本地已接受但未提交的脏写。
4. 复制在文件布局之下完成。布局层请求持久 chunk，并在收到可验证 receipt 后让 Meta 提交新版本。
5. OwnerFs 和 DFS 共用进程、FUSE 和传输基础设施，但挂载、后端状态机、缓存策略和验收结论相互独立。

## 入口

| 入口 | 后端 | 说明 |
| --- | --- | --- |
| FUSE 挂载 | OwnerFs 或 DFS | 两类后端使用独立挂载会话 |
| DFS SDK | DFS | 后续高性能 DFS 入口，不是 OwnerFs 通用 API |
| 后续块设备适配 | DFS | 基础镜像走固定版本读，写层产生新 chunk |

## 组件职责

| 组件 | 职责 |
| --- | --- |
| `afs-meta` | 命名空间、inode、写租约、文件版本、布局根、放置快照、副本目录和幂等提交结果 |
| `afs-node` | FUSE 会话、DFS dirty 状态、OwnerFs Home 访问、chunk 存储、复制执行、远端读、验证缓存和 spill worker |
| 本地存储 | 暂存字节、digest 校验、持久发布、本地目录和 reader pin |
| 节点传输 | 节点控制、复制传输、固定版本范围读和 OwnerFs 远端访问 |
| 外部 spill | 可选冷容量；写入、校验和 Meta 提交后才可作为来源 |

## 当前交付边界

架构页描述接受的目标设计；当前版本是否已经实现、是否通过验收，以 [当前计划](development/plan.md) 和 [状态摘要](status.md) 为准。

当前可试用路径只要求中心 Meta 的 `local-file` 后端具备重启恢复；`memory` 是一次性演示后端。etcd、Redis、多 Meta、高可用、复杂可靠性、RDMA、spill、DFS SDK 和大规模长时运行均后置。

## 机制页

| 契约 | 页面 |
| --- | --- |
| 对象模型、稀疏文件和 `LayoutRoot` | [数据模型](architecture/data-model.md) |
| 写可见性、sync、同步写和不确定提交 | [写语义](architecture/write-semantics.md) |
| 副本策略和 R=1/R=N 路径 | [复制](architecture/replication.md) |
| chunk finalization、布局 COW 和恢复 | [本地存储与 COW](architecture/local-storage.md) |
| 固定版本读、缓存、seed lease 和 spill | [读缓存与 spill](architecture/read-cache-spill.md) |
| OwnerFs Home、远端访问和 workspace bind | [OwnerFs](architecture/ownerfs.md) |
| Meta 提交权威、读视图和后端事务边界 | [Meta](architecture/meta.md) |
| 代码模块入口 | [模块地图](architecture/module-map.md) |
