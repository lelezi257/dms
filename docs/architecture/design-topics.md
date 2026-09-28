# AFS 架构设计专题

状态：Research Agenda  
实现状态：Not Implemented  
权威合同：[架构原则](../../PRINCIPLES.md) · [架构总览](overview.md) · [数据 Profile](profiles.md)

## 目的

本文维护 Distributed BlobFs 的设计专题、研究顺序和文档状态。每个专题使用独立设计文档承载术语、外部语义、数据模型、状态机、故障矩阵、备选方案和验收标准。影响外部语义、持久格式或跨模块合同的稳定结论必须进入 RFC；研究文档本身不构成已实现能力。

## 共同前提

- AFS 对普通应用提供通用 POSIX Namespace。
- Distributed BlobFs 是通用分布式主线；OwnerFs 是 1～4 节点 Agent Workspace 的特化后端。
- Mutable 与 Published Immutable Profile 共用 Namespace、Meta、Storage Service、Chunk Store、placement、transport 和运维体系，使用各自的写入状态机。
- Blob 是否直接暴露用户 API 仍待设计；不可变镜像、Snapshot 和 Checkpoint 是重点优化负载。
- 单副本本地写是基础能力，多副本、异步副本和 spill 是可配置策略。
- `write`、数据提交、文件元数据同步、介质持久化、`seal` 和 `publish` 是不同完成边界。
- 小数据可以随控制消息 inline；大数据通过共享内存、注册 buffer、RDMA 或流式传输搬运，协议不绑定单一 transport。

## 专题索引

| 顺序 | 专题 | 状态 | 主要产物 |
| --- | --- | --- | --- |
| 1 | [File、Blob、Chunk 统一数据模型](01-file-blob-chunk-model.md) | Research | 稳定身份、引用关系、Blob API 选择、stripe/chain 布局 |
| 2 | [写入完成、持久化与发布语义](02-write-durability-publication.md) | Research | `write/flush/fsync/seal/publish` 语义矩阵和完成级别 |
| 3 | [单副本与多副本写入状态机](03-replication-state-machine.md) | Research | R=1/R=N 统一流程、故障和 chain 重配置 |
| 4 | [本地 ChunkEngine 与 COW](04-local-chunk-engine-cow.md) | Research | pending/commit、物理 COW、manifest COW、恢复与回收 |
| 5 | [文件长度、truncate 与 Blob seal](05-length-truncate-seal.md) | Research | length 水位、truncate version、稳定切点和发布门禁 |
| 6 | [可靠性与高性能数据路径](06-reliability-performance-path.md) | Research | 幂等、版本、checksum、inline/SHM/RDMA、P2P 和 spill |

## 文档成熟流程

```text
Research
  → 形成候选模型与未决问题
Draft RFC
  → 固化外部语义、持久格式和跨模块合同
Accepted RFC
  → 允许进入实现计划
Implemented
  → 由代码、故障验证和性能证据闭合
```

每个专题完成时至少交付：

1. 用户 Case、目标和非目标；
2. 术语、稳定身份和持久字段；
3. 正常状态机与调用路径；
4. 并发、幂等和故障矩阵；
5. 备选方案及取舍；
6. 功能、故障、性能和运维验收标准；
7. 需要新增或更新的 RFC、代码模块和实验。

## 当前入口

首先完成[专题一](01-file-blob-chunk-model.md)：确定 File、Blob、Chunk、Manifest、Stripe、Replica Chain 和 Physical Position 的层次，以及 Blob 采用公开 API、POSIX 映射还是双入口。专题二以专题一的对象身份和生命周期为输入。
