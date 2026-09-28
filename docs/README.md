# AFS 文档

## 开始阅读

1. [产品定位](product-positioning.md)：目标用户、核心价值、产品结构和适用范围。
2. [架构原则](../PRINCIPLES.md)：长期稳定的产品与架构合同。
3. [架构总览](architecture/overview.md)：Meta、Node、OwnerFs、BlobFs、Storage Service 和外部存储的关系。
4. [数据 Profile](architecture/profiles.md)：OwnerFs、Mutable 与 Published Immutable 的边界。
5. [架构设计专题](architecture/design-topics.md)：Distributed BlobFs 六个设计专题、顺序、状态和独立文档入口。
6. [副本和缓存状态](semantics/copy-states.md)：Staging、Cache、Replica、External 和发布/逐出门禁。
7. [当前状态](current-status.md)：已经实现、实验可用、计划和研究中的能力。
8. [Roadmap](../ROADMAP.md)：纵向 Milestone 和目标 E2E。
9. [参与贡献](../CONTRIBUTING.md)：RFC、任务、实现和验证要求。

## 设计与语义

- [RFC 索引](rfcs/README.md)
- [RFC-0001：产品架构与数据 Profile](rfcs/0001-product-architecture.md)
- [错误合同](error-contract.md)
- [代码地图](code-layout.md)

## 运行与验证

- [运行指南](foundation-running.md)
- [详细实现状态与实验记录](status.md)
- [实现任务入口](next.md)
- [OwnerFs P2P 并发优化](plans/2026-09-27-ownerfs-p2p-concurrency.md)
- [MetaStore 提交边界](plans/2026-09-27-meta-store.md)
- [OwnerFs 根恢复审视](reviews/ownerfs-v16-root-recovery.md)

## 文档状态

| 状态 | 含义 |
| --- | --- |
| `Implemented` | 代码和验证证据已经闭合 |
| `Experimental` | 真实链路可运行，产品边界或规模尚未闭合 |
| `Accepted Design` | 架构合同已确认，代码可以尚未实现 |
| `Draft` | 内容仍可调整，不作为实现合同 |
| `Research` | 独立研究或实验存在，产品路径尚未接入 |
| `Planned` | Roadmap 中的目标能力 |
| `Superseded` | 由现行文档替代，仅作历史参考 |

## 历史参考

- [需求分析 HTML](requirements.html)：Status `Superseded`。
- [详细架构 HTML](architecture.html)：Status `Superseded`。
- [工作负载速览](workloads.md)：Status `Superseded`。

现行产品语义以 `PRINCIPLES.md`、`product-positioning.md`、`architecture/` 和 Accepted RFC 为准。历史文档不定义当前产品边界。
