# 实现任务

工程主线以 [Roadmap](../ROADMAP.md) 为准。当前优先级是建立最小 Distributed BlobFs，再补齐 Mutable Profile、Published Immutable Profile 和多源读取。

## 设计先行入口

[架构设计专题](architecture/design-topics.md)保存 Distributed BlobFs 的六个设计专题及其依赖顺序。当前先完成[专题一：File、Blob、Chunk 统一数据模型](architecture/01-file-blob-chunk-model.md)，明确对象层次、稳定身份、Blob API 是否公开、stripe/replica chain 含义和引用/GC 边界。专题结论影响外部语义、持久格式或跨模块合同时，必须形成 Draft RFC 并完成评审；研究文档不直接作为实现合同。

专题一完成后依次推进写入完成语义、副本状态机、本地 COW、长度与 seal、可靠性与高性能路径。各专题文档会逐步收敛为对应设计文档和 RFC，下面的实现任务引用已接受的结论执行。

## P0：最小 Distributed BlobFs

1. 以 RFC 固化 inode、extent/chunk、文件版本、layout epoch、placement 和副本状态模型。
2. 实现 Meta 侧文件布局事务，以及 Node 侧 chunk 创建、写入、读取、校验和删除。
3. 打通 FUSE `create → write → fsync → close → open → read` 的两节点端到端路径。
4. 定义副本确认规则、失败重试、重启恢复和校验失败处理。
5. 为每个状态转换补充可观测字段，使实现状态可映射到[副本与缓存状态](semantics/copy-states.md)。

## P1：Mutable Profile

1. 定义并实现并发读写、truncate、rename、unlink、fsync 与崩溃恢复语义。
2. 以一致性协议维护权威副本集合；多源读取只能选择同一已提交版本。
3. 补 Meta 单活动围栏、选主和故障注入证据。

## P1：OwnerFs 完整性

1. 补齐 Agent 常用 `chmod`、`chown`、`atime`、`mtime`。
2. 实现根删除、同名重建和跨节点根列举。
3. 扩展文件大小、数量、并发和远端读写比例矩阵，记录 B 重读与覆盖写热点。

## P2：Published Immutable Profile

1. 定义显式 Snapshot/Publish API；`fsync` 只保证文件持久性，不承担发布语义。
2. 生成内容校验、不可变版本和可验证 manifest。
3. 将已验证本地副本、近端 cache 和外部对象存储副本纳入统一 placement。
4. 实现 authoritative replica 与 verified cache 的多源调度、回源、修复和 GC。

## P3：高性能与容量层

1. 接通 Native SDK 的批量 I/O、SHM 与可选 RDMA 数据路径。
2. 实现本地 SSD、NVMe、HDD 的容量与热度管理。
3. 实现可选对象存储 spill，并验证回源、校验、删除与灾难恢复。

贡献前先读[贡献指南](../CONTRIBUTING.md)和[RFC 规则](rfcs/README.md)。所有能力声明必须附带源码、测试或实验位置。
