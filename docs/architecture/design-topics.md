# AFS 架构设计专题

状态：Research Agenda
实现状态：Not Implemented
权威合同：[架构原则](../../PRINCIPLES.md) · [架构总览](overview.md) · [工作负载路径](profiles.md)

## 目的

本文维护 DistributedFs 的设计专题、依赖顺序和文档状态。每个专题使用独立设计文档承载用户 Case、术语、数据模型、状态机、RPC 预算、故障矩阵、备选方案和验收标准。影响外部语义、持久格式或跨模块合同的稳定结论必须进入 RFC。

## 共同前提

- AFS 对普通应用提供通用 POSIX Namespace。
- DistributedFs 是通用分布式主线；OwnerFs 是 1～4 节点 Agent Workspace 的特化后端。
- DFS 使用 `InodeRecord → FileVersion → LayoutRoot/ExtentMap → ChunkObject` 统一表达普通文件与固定版本负载。
- 提交后的 FileVersion、LayoutRoot 和 ChunkObject 不可变；文件可变性来自 Head 切换和 Extent Overlay。
- 不引入必需 Blob API；Pin、Alias 和 RootManifest 是可选业务能力。
- 单副本本地写是基础能力，多副本、异步副本和 Spill 是 ChunkStore 的可配置策略。
- `write`、Chunk Finalize、DurabilityPolicy、FileVersion CAS、`fsync` 和业务 Snapshot 是不同完成边界。
- 小数据可以随控制消息 Inline；大数据通过共享内存、注册 Buffer、RDMA 或流式传输搬运，协议不绑定单一 Transport。

## 专题索引

| 顺序 | 专题 | 状态 | 主要产物 |
| --- | --- | --- | --- |
| 1 | [FileVersion、Extent 与 Chunk 数据模型](01-file-version-chunk-model.md) | Accepted Design | 对象身份、三个 E2E、RPC 预算和模块边界；[RFC-0002](../rfcs/0002-file-version-chunk-model.md) |
| 2 | [写入完成、持久化与可见性](02-write-durability-publication.md) | Research | `write/flush/fsync/O_SYNC`、版本 CAS 和跨节点可见性 |
| 3 | [单副本与多副本写入状态机](03-replication-state-machine.md) | Research | R=1/R=N、ChunkReceipt、故障和 Chain 重配置 |
| 4 | [本地 ChunkEngine 与 COW](04-local-chunk-engine-cow.md) | Research | StagedChunk/Finalize、Patch、Compaction、恢复与回收 |
| 5 | [文件长度、truncate 与稳定版本](05-length-truncate-seal.md) | Research | Length 水位、Append、truncate、Pin 与 RootManifest |
| 6 | [可靠性与高性能数据路径](06-reliability-performance-path.md) | Research | 幂等、校验、Inline/SHM/RDMA、P2P、Cache 和 Spill |

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

每个专题至少交付：

1. 用户 Case、目标和非目标；
2. 术语、稳定身份和持久字段；
3. 正常状态机与端到端调用路径；
4. Meta 事务、逻辑 RPC、连接和 Data Hop 预算；
5. 并发、幂等和故障矩阵；
6. 抽象保留、合并或删除的依据；
7. 功能、故障、性能和运维验收标准。

## 当前入口

[专题一](01-file-version-chunk-model.md)及 [RFC-0002](../rfcs/0002-file-version-chunk-model.md)已经接受，固定 DFS 的对象关系、三个 E2E、R=1/R=N 分层与 RPC 原则。下一专题在此基础上定义普通 write、O_SYNC、flush、fsync、跨节点可见性和并发 Writer 合同。
