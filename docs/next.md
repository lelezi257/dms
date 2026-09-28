# 实现任务

工程主线以 [Roadmap](../ROADMAP.md) 为准。当前优先级是完成 DFS 写入语义设计，再实现最小的 FileVersion/Extent/Chunk 纵向链路。

## 设计入口

[专题一](architecture/01-file-version-chunk-model.md)及 [RFC-0002](rfcs/0002-file-version-chunk-model.md)已经接受，确定以下实现合同：

- inode 通过可变 `head_version` 指向已提交的不可变 `FileVersion`；
- `FileVersion → LayoutRoot/ExtentMap → Extent → ChunkObject` 构成统一事实源；
- `StagedChunk` 只有 finalize 后才成为可读的 `ChunkObject`；
- 单副本和多副本只在 `ChunkStore::put` 以下分叉，上层只消费 `ChunkReceipt`；
- `fsync` 强制形成完整、持久的 FileVersion，不自动创建业务 Alias、Pin 或 RootManifest；
- 固定版本多源读取先固定 `FileVersionId`，再从合格副本或缓存读取其 Chunk。

当前进入[专题二](architecture/02-write-durability-publication.md)，重点固化普通 write/flush/fsync 的完成语义、跨节点可见性、错误返回和所需 RPC。

## P0：完成写入语义设计

1. 定义 write、flush、fsync、close 各自对客户端缓冲、Chunk 提交、FileVersion 提交和耐久性的承诺。
2. 确定多写场景的并发排序与跨节点可见机制。
3. 明确 inode `head_version` 的 CAS、失败重试、幂等键和故障恢复。
4. 为单副本、多副本、4 KiB 覆盖写分别列出正常及故障 RPC 时序。

## P1：最小 DistributedFs

1. 实现 `StagedChunk → ChunkObject` 和本地 `ChunkStore::put/get`。
2. 实现 `ExtentMap`、`LayoutRoot`、`FileVersion` 与 inode head 提交事务。
3. 打通 FUSE `create → write → fsync → close → open → read` 单节点端到端路径。
4. 增加 R=3 的 `ChunkStore::put` 实现，不改变文件层接口。
5. 以两节点读取与单节点故障换源验证固定版本读取。

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
