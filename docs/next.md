# 实现任务

工程主线以 [Roadmap](../ROADMAP.md) 为准。最小 R=1 FileVersion/Extent/Chunk 纵向链路已经实现。当前优先级是固化写入语义，并把首阶段实现替换成可扩展的并发、摘要和布局机制。

[README 目标架构图](../README.md#架构)已作为总览入口；它不改变下述工程任务和验收顺序。

## 设计入口

[专题一](architecture/01-file-version-chunk-model.md)、[RFC-0002](rfcs/0002-file-version-chunk-model.md)、[专题二](architecture/02-write-durability-publication.md)、[RFC-0003](rfcs/0003-write-visibility-durability.md)、[专题三](architecture/03-replication-state-machine.md)及 [RFC-0004](rfcs/0004-replication-state-machine.md)已经接受，确定以下实现合同：

- inode 通过可变 `head_version` 指向已提交的不可变 `FileVersion`；
- `FileVersion → LayoutRoot/ExtentMap → Extent → ChunkObject` 构成统一事实源；
- 普通 write 由 inode owner 排序并进入共享 `InodeWriteState/DirtyExtentMap`，不修改 committed FileVersion；
- `DfsWriteSession` 只保存一次 open 的 flags、水位和错误游标；
- CommitTrigger 冻结写入前缀，`StagedChunk` 只在 ChunkStore 内部存在，finalize 后才成为可读的 `ChunkObject`；
- 单副本和多副本只在 `ChunkStore::put` 以下分叉，上层只消费 `ChunkReceipt`；
- `fdatasync` 提交数据与恢复索引，`fsync` 再提交完整 inode 属性；两者不自动创建业务 Alias、Pin 或 RootManifest；
- FUSE flush/release 不替代同步合同；后台 writeback 可以提交版本但不产生用户可依赖的完成点；
- 文件同步与目录项 `fsync(dir)` 是两个合同；
- 固定版本多源读取先固定 `FileVersionId`，再从合格副本或缓存读取其 Chunk。
- R=1 使用无 Peer 的 Local Fast Path；RN 由 ReplicationEngine 协调多个共享 LocalChunkStore 的副本。
- 副本数量由文件系统初始化时的 `desired_copies/sync_required_copies` 固定；Meta 维护 Placement 权威，Node 从缓存快照生成 ReplicationPlan。
- ReplicaAck 是 Peer wire 证明，ChunkReceipt 是提交层聚合证明，CopyRecord 是 Meta 长期目录事实。
- 异步补副本任务与 FileVersion 在同一 Meta 事务登记；欠副本不阻止从现存有效 Copy 读取。

下一设计入口是[专题四](architecture/04-local-chunk-engine-cow.md)。RFC-0004 的副本基础类型、Meta Placement 合同、Node ReplicationEngine 与 Peer Chunk RPC 形状已经落入代码；下一工程入口是先完成专题四，再实现 RN 目标端 staging/finalize、幂等 ACK 与多节点 placement，避免在本地 Chunk crash contract 未固定前完成远端复制。

RPC 物理布局保持为 `control.rs`、`data.rs`、`meta.rs`、`peer.rs`。R=N 设计可以在 `data.rs`/`peer.rs` 内增加 Chunk 协议实现；全部专项收敛前不按 OwnerFs/DFS 拆子文件。

## P0：固化写入语义并消除首阶段限制

1. 已接入 WriteLease、lease epoch fencing 和组合 `OpenWrite` Meta 操作；下一步实现跨节点 owner routing。
2. 已将 dirty data 从 DfsWriteSession 移到每 inode 的 InodeWriteState，并实现本地跨 handle 的 base+overlay 读取；下一步扩展到远端 owner 转发。
3. 已实现 WriteSeq、CommitBatch、同步水位、定时后台 writeback、优雅退出 drain 与 sticky error；下一步补故障注入、每 inode 非阻塞 freeze 和错误恢复矩阵。
4. 已分离 write、flush、fdatasync、fsync 和 release，并接通 `O_DSYNC/O_SYNC`；下一步实现目录 `fsync(dir)`。
5. 已把 inode `head_version` CAS、lease 校验和 commit 幂等键接入 Meta；下一步补充失败重试和恢复测试矩阵。
6. 已避免全局 handle table 锁跨磁盘 I/O 与 Meta RPC；下一步缩短 inode 级写状态锁的 commit 临界区。
7. 选定带算法版本的强内容摘要，并定义旧 Chunk 格式的兼容边界。
8. 为单副本、多副本、4 KiB 覆盖写分别列出正常及故障 RPC 时序。

## P1：扩展 DistributedFs

1. 将当前 whole-file 单 Chunk 实现扩展为分块、Extent 树、覆盖写与 compaction。
2. 补齐 truncate、append、rename、unlink、目录与打开句柄语义。
3. 已增加 N/M `ReplicationConfig`、`DfsChunkStore` R1/RN 分叉、PlacementSnapshot、ReplicationEngine、ACK/receipt 与 Meta 提交框架；下一步实现 RN transport 和后台 ReplicationTask worker。
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
