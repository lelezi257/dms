# 实现状态

更新时间：2026-09-27。

AFS 是面向业务集群近计算场景的通用 POSIX 分布式文件系统。产品包含两条数据后端：Distributed BlobFs 是通用分布式主线，OwnerFs 是 1～4 节点一体机 Agent workspace 的专用优化。BlobFs 同时承载可变文件与已发布不可变数据，两类 Profile 共享 namespace、Meta、Storage Service、placement、transport 和运维体系，并使用各自的写入状态机。

## 当前能力

| 能力 | 状态 | 已验证边界 |
| --- | --- | --- |
| CLI、TOML、日志、metrics、trace、错误框架 | 已实现 | Linux 构建与现有测试通过 |
| FUSE、SDK、REST 入口 | 已实现基础框架 | OwnerFs 路径已投入三 VM 验收 |
| MetaStore | 已实现单活动基础 | etcd、local-file、memory；后端 ACK 后发布可见状态 |
| OwnerFs | 已实现阶段能力 | 本机普通文件、跨 Node P2P、授权校验、句柄回收 |
| Distributed BlobFs | 骨架 | 通用分布式 extent/chunk 数据面尚未实现 |
| Mutable Profile | 未实现 | 多读多写、一致性和故障恢复合同待 RFC 固化 |
| Published Immutable Profile | 未实现 | publish、校验、cache、P2P 多源和 GC 待实现 |
| 外部对象存储 spill | 未实现 | 属于容量层选项，不是系统成立条件 |
| Native SDK 高性能数据面 | 基础框架 | SHM/RDMA 文件内容路径尚未接通 |

## OwnerFs 证据

最终 Linux Release 二进制在三 VM 功能验收中通过 15/15。带 `fh` 的属性操作与同句柄 I/O 保序；Home 校验打开句柄的实际根、peer 和授权；异步 RELEASE 支持有界重试；远端 daemon 异常退出后，Home 在 Meta 会话失效后回收遗留句柄。

固定 200×4 KiB、8 worker 负载下，完整远端 W2 两份 12 轮结果为 MooseFS 的 **0.768/0.780**；W1 本机私有根为 **0.482/0.451**；顺序 W2 为 **1.106/1.080**。这些结果只证明该固定负载，不代表完整 POSIX 或所有工作负载均优于 MooseFS。详细证据见[OwnerFs 修复与复验](reviews/2026-09-27-ownerfs-p2p-hardening.md)。

## 已知边界

- Meta 单活动围栏与选主尚未完成，不能宣称生产 HA。
- OwnerFs 尚缺根删除、跨节点根列举及部分常用属性操作。
- `memory` MetaStore 只用于可丢弃测试；`local-file` 只承诺单机单盘。
- RDMA 已有传输探测能力，文件内容仍走 gRPC P2P。
- 完整 POSIX、VM 掉电、长稳、容量压力和对象存储 spill 尚未验收。

产品合同见[产品定位](product-positioning.md)、[架构总览](architecture/overview.md)和[架构原则](../PRINCIPLES.md)。工程优先级见[Roadmap](../ROADMAP.md)与[实现任务](next.md)。
