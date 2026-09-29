# AFS 架构设计专题

状态：Research Agenda
实现状态：专题一、二的 R1 纵向链路、专题三副本框架和专题四 R1 热路径框架已实现；专题五已归并到专题一、二、四；RN 远端搬运、专题四 recovery/GC 闭环及专题六继续推进
权威合同：[架构原则](../../PRINCIPLES.md) · [架构总览](overview.md) · [工作负载路径](profiles.md)

## 目的

本文维护 DistributedFs 的设计专题、依赖顺序和文档状态。每个专题使用独立设计文档承载用户 Case、术语、数据模型、状态机、RPC 预算、故障矩阵、备选方案和验收标准。影响外部语义、持久格式或跨模块合同的稳定结论必须进入 RFC。

## 共同前提

- AFS 对普通应用提供通用 POSIX Namespace。
- DistributedFs 是通用分布式主线；OwnerFs 是 1～4 节点 Agent Workspace 的特化后端。
- DFS 使用 `InodeRecord → FileVersion → LayoutRoot/ExtentMap → ChunkObject` 统一表达普通文件与固定版本负载。
- 提交后的 FileVersion、LayoutRoot 和 ChunkObject 不可变；文件可变性来自 Head 切换和 Extent Overlay。
- 不引入必需 Blob API；Pin、Alias 和 RootManifest 是可选业务能力。
- 单副本本地写是基础能力，多副本、异步副本和 Spill 是 ChunkStore 的执行能力；副本数量由文件系统初始化时的 ReplicationConfig 固定。
- 普通 `write` 的 dirty 可见性、Chunk Finalize、ReplicationSatisfied、FileVersion CAS、`fdatasync/fsync` 和业务 Snapshot 是不同完成边界。
- WriteLease 指定活跃 inode owner；DfsWriteSession 只保存 handle 状态，dirty data 属于 inode 共享的 InodeWriteState。
- 小数据可以随控制消息 Inline；大数据通过共享内存、注册 Buffer、RDMA 或流式传输搬运，协议不绑定单一 Transport。

## 专题索引

| 顺序 | 专题 | 状态 | 主要产物 |
| --- | --- | --- | --- |
| 1 | [FileVersion、Extent 与 Chunk 数据模型](01-file-version-chunk-model.md) | Accepted Design | 对象身份、三个 E2E、RPC 预算和模块边界；[RFC-0002](../rfcs/0002-file-version-chunk-model.md) |
| 2 | [写入完成、持久化与可见性](02-write-durability-publication.md) | Accepted Design | 用户/Node/Meta 时间线、WriteLease、dirty visibility、同步合同和 [RFC-0003](../rfcs/0003-write-visibility-durability.md) |
| 3 | [单副本与多副本写入状态机](03-replication-state-machine.md) | Framework Implemented | 文件系统级 N/M 配置、R1/RN 分叉、Placement、ACK、异步任务和 [RFC-0004](../rfcs/0004-replication-state-machine.md) |
| 4 | [本地 ChunkEngine 与 COW](04-local-chunk-engine-cow.md) | Framework Implemented | Layout COW、Local finalize、Physical COW、恢复、GC 与 [RFC-0005](../rfcs/0005-local-chunk-engine-cow.md) |
| 5 | [文件长度、truncate 与稳定版本](05-length-truncate-seal.md) | Merged | length、implicit hole、append、truncate 已归并到专题一、二、四，不增加独立抽象 |
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

[专题一](01-file-version-chunk-model.md)、[专题二](02-write-durability-publication.md)、[专题三](03-replication-state-machine.md)、[专题四](04-local-chunk-engine-cow.md)及对应 RFC 已经接受。原专题五的 length、implicit hole、append 和 truncate 结论已经归并到专题一、二、四，不建立独立模块。代码已经实现独立 DFS mount、R1 本机不可变 Chunk、inode owner dirty view、同步边界、FileVersion CAS、副本框架，以及专题四的分块 CommitPlanner、base Chunk 继承、LocalChunkRecord 和批量可恢复 finalize。专题四剩余 Pack/relocation、pin/删除状态机、orphan reconciliation、compaction policy 和故障验证。下一设计入口是[专题六](06-reliability-performance-path.md)。
