# 实现任务

工程主线以 [Roadmap](../ROADMAP.md) 为准。最小 R=1 FileVersion/Extent/Chunk 纵向链路已经实现。当前优先级是固化写入语义，并把首阶段实现替换成可扩展的并发、摘要和布局机制。

## 设计入口

[专题一](architecture/01-file-version-chunk-model.md)及 [RFC-0002](rfcs/0002-file-version-chunk-model.md)已经接受，确定以下实现合同：

- inode 通过可变 `head_version` 指向已提交的不可变 `FileVersion`；
- `FileVersion → LayoutRoot/ExtentMap → Extent → ChunkObject` 构成统一事实源；
- `StagedChunk` 只有 finalize 后才成为可读的 `ChunkObject`；
- 单副本和多副本只在 `ChunkStore::put` 以下分叉，上层只消费 `ChunkReceipt`；
- `fsync` 强制形成完整、持久的 FileVersion，不自动创建业务 Alias、Pin 或 RootManifest；
- 固定版本多源读取先固定 `FileVersionId`，再从合格副本或缓存读取其 Chunk。

当前进入[专题二](architecture/02-write-durability-publication.md)，重点固化普通 write/flush/fsync 的完成语义、跨节点可见性、错误返回和所需 RPC。

## P0：固化写入语义并消除首阶段限制

1. 定义 write、flush、fsync、close 各自对客户端缓冲、Chunk 提交、FileVersion 提交和耐久性的承诺。
2. 确定多写场景的并发排序与跨节点可见机制。
3. 明确 inode `head_version` 的 CAS、失败重试、幂等键和故障恢复。
4. 将全局 handle table 临界区改为每句柄并发控制，磁盘 I/O 与 Meta RPC 不占用全局锁。
5. 选定带算法版本的强内容摘要，并定义旧 Chunk 格式的兼容边界。
6. 为单副本、多副本、4 KiB 覆盖写分别列出正常及故障 RPC 时序。

## P1：扩展 DistributedFs

1. 将当前 whole-file 单 Chunk 实现扩展为分块、Extent 树、覆盖写与 compaction。
2. 补齐 truncate、append、rename、unlink、目录与打开句柄语义。
3. 增加 R=3 的 `ChunkStore::put` 实现，不改变文件层接口。
4. 以两节点读取与单节点故障换源验证固定版本读取。

## P1：OwnerFs 完整性

1. 补齐 Agent 常用 `chmod`、`chown`、`atime`、`mtime`。
2. 实现根删除、同名重建和跨节点根列举。
3. 扩展文件大小、数量、并发和远端读写比例矩阵。

## P2：固定版本优化

1. 实现按 `FileVersionId` 的 Alias、Pin/Retention 和 RootManifest 可选能力。
2. 实现已验证缓存、多源调度、消费者转 seed、回源、修复和 GC。
3. 对镜像、Snapshot 和 Checkpoint 验证 range read 与大规模启动。

## P3：高性能与容量层

1. 接通 Native SDK 的批量 I/O、SHM 与可选 RDMA 数据路径。
2. 实现本地 SSD、NVMe、HDD 的容量与热度管理。
3. 实现可选对象存储 spill，并验证回源、校验、删除与灾难恢复。

贡献前先读[贡献指南](../CONTRIBUTING.md)和[RFC 规则](rfcs/README.md)。所有能力声明必须附带源码、测试或实验位置。
