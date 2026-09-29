# AFS 工作负载路径

状态：Accepted Design
实现状态：OwnerFs Experimental；DistributedFs R=1 Experimental

## 结构

```text
AFS Namespace
├── OwnerFs Backend
│   └── 1～4 节点 Workspace
└── DistributedFs Backend
    └── 统一 FileVersion / LayoutRoot / ChunkObject 数据模型
        ├── 普通可变文件
        └── 固定版本优化负载
```

普通文件和固定版本不是两套 Profile。两者使用相同的 InodeRecord、FileVersion、LayoutRoot、ChunkStore、placement、transport 和运维体系。差异来自版本保留、读取来源和调度策略。

## 普通可变文件

适用范围：

- 通用共享文件；
- 多客户端读写；
- overwrite、append、truncate 和 sparse file；
- 不适合放在单 Home 的规模化 Workspace。

行为：

- inode owner 将普通写入排序到共享 InodeWriteState/DirtyExtentMap；
- DfsWriteSession 只保存一次 open 的 flags、水位和错误观察位置；
- 同步或后台 CommitTrigger 提交新的不可变 FileVersion；
- 未修改范围复用旧 Chunk；
- 小范围修改使用 Patch Chunk 和 Extent Overlay；
- Compaction 控制 Overlay 深度；
- 并发 Writer、Append、truncate 和跨节点可见性由一致性协议管理。

## 固定版本优化负载

适用范围：

- OCI 镜像和文件树；
- Nydus/EROFS/OverlayBD 数据；
- MicroVM 根磁盘；
- Agent Workspace Snapshot；
- Checkpoint、模型、数据集和只读构建产物。

行为：

- 固定一个或多个 FileVersion；
- Pin 防止版本和 Chunk 被 GC；
- Alias 提供稳定业务名称；
- RootManifest 组合多文件一致视图；
- 读取可选择 Durable Replica、Verified Cache、P2P Seed 和 ExternalCommitted；
- 消费者完成 Chunk 校验后可以成为 Cache Seed；
- FileVersion、LayoutRoot 和 ChunkObject 不因副本位置变化而变化。

`fdatasync/fsync` 提交文件版本，后台 writeback 也可以生成内部版本；这些操作都不自动 Pin、不创建 Alias，也不构造多文件 RootManifest。

## OwnerFs

适用范围：

- 1～4 节点；
- 一体机式部署；
- Agent Workspace；
- 大多数操作发生在 Home；
- 调度器可以维持计算与 Home 亲和。

数据路径：

- Home 使用本地普通文件系统；
- 本机访问不分 Chunk；
- 远端访问回到 Home；
- Meta 管理 WorkspaceRoot、Home、授权和会话；
- OwnerFs 与 DFS 当前独立运行，不定义自动转换路径。

边界：

- Home 永久丢盘不自动由缓存接管；
- 不提供通用多副本写；
- 不提供跨大量节点聚合文件带宽；
- OwnerFs 与 DFS 之间的 rename/link 返回明确错误；
- 共享基础设施不合并两种后端的文件状态机。

## 入口选择

| 条件 | 推荐路径 |
| --- | --- |
| 1～4 节点 Agent Workspace，本地亲和明显 | OwnerFs |
| 通用共享可变文件 | DistributedFs |
| 大文件跨节点并行 I/O | DistributedFs + Native SDK |
| 镜像、Snapshot、Checkpoint | DFS 固定 FileVersion + Pin/Alias |
| MicroVM 基础磁盘 | DFS 固定 FileVersion + Block Adapter |
