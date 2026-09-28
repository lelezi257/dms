# 实现状态

更新时间：2026-09-28。

AFS 是面向业务集群近计算场景的通用 POSIX 分布式文件系统。产品包含两条后端：`DistributedFs`（简称 `DFS`）是通用分布式主线，`OwnerFs` 是 1～4 节点一体机 Agent workspace 的专用优化。

DFS 使用统一的数据模型承载普通可变文件和镜像、Snapshot 等固定版本工作负载：文件可变性由 inode 指向哪个 `FileVersion` 表达；已提交的 `FileVersion`、`LayoutRoot`、`ExtentMap` 和 `ChunkObject` 都不可变。固定版本读取、多源 P2P、缓存与 spill 直接复用这套事实源，不建立第二套 Blob 数据模型。

## 架构设计专题

[架构设计专题](architecture/design-topics.md)维护 DFS 的六个设计专题及其依赖关系。

专题一已经接受：[FileVersion、Extent 与 Chunk 数据模型](architecture/01-file-version-chunk-model.md)和 [RFC-0002](rfcs/0002-file-version-chunk-model.md)定义了统一不可变版本模型、`fsync` 边界、单副本与多副本分叉点、三个端到端 Case 和 RPC 预算。当前进入专题二：普通写入的完成、持久化和跨节点可见性。

目标接入模型已经固定为两个独立 mount：OwnerFs 与 DFS 分别建立 FuseSession、FUSE connection、inode/handle table 和缓存策略，只复用 `fuse` 模块代码与 `Backend` 接口。`DfsWriteSession` 是 DFS 专属类型；OwnerFs 不进入 FileVersion/Extent/Chunk 写入状态机。当前源码仍是单 mount、多 namespace 骨架，尚未迁移到目标结构。

## 当前能力

| 能力 | 状态 | 已验证边界 |
| --- | --- | --- |
| CLI、TOML、日志、metrics、trace、错误框架 | 已实现 | Linux 构建与现有测试通过 |
| FUSE、SDK、REST 入口 | 已实现基础框架 | OwnerFs 路径已投入三 VM 验收 |
| MetaStore | 已实现单活动基础 | etcd、local-file、memory；后端 ACK 后发布可见状态 |
| OwnerFs | 已实现阶段能力 | 本机普通文件、跨 Node P2P、授权校验、句柄回收 |
| DistributedFs | 骨架 | 通用 extent/chunk 数据面尚未实现；现有源码路径仍暂名 `blobfs.rs` |
| FileVersion 数据模型 | Accepted Design | RFC-0002 已接受，代码尚未实现 |
| 跨节点写入可见性 | Research | 由专题二确定 lease、路由或 sequencer 合同 |
| 固定版本多源读取 | Accepted Design | 身份和读取规则已确定，调度与数据路径尚未实现 |
| 外部对象存储 spill | 未实现 | 属于容量层选项，不是系统成立条件 |
| Native SDK 高性能数据面 | 基础框架 | SHM/RDMA 文件内容路径尚未接通 |

## OwnerFs 证据

最终 Linux Release 二进制在三 VM 功能验收中通过 15/15。带 `fh` 的属性操作与同句柄 I/O 保序；Home 校验打开句柄的实际根、peer 和授权；异步 RELEASE 支持有界重试；远端 daemon 异常退出后，Home 在 Meta 会话失效后回收遗留句柄。

固定 200×4 KiB、8 worker 负载下，完整远端 W2 两份 12 轮结果为 MooseFS 的 **0.768/0.780**；W1 本机私有根为 **0.482/0.451**；顺序 W2 为 **1.106/1.080**。这些结果只证明该固定负载，不代表完整 POSIX 或所有工作负载均优于 MooseFS。详细证据见[OwnerFs 修复与复验](reviews/2026-09-27-ownerfs-p2p-hardening.md)。

## 已知边界

- Meta 单活动围栏与选主尚未完成，不能宣称生产 HA。
- OwnerFs 尚缺根删除、跨节点根列举及部分常用属性操作。
- DFS 的 FileVersion、ExtentMap、ChunkStore 和副本协议尚未实现。
- 严格普通写跨节点可见性仍待专题二固化。
- RDMA 已有传输探测能力，文件内容仍走 gRPC P2P。
- 完整 POSIX、VM 掉电、长稳、容量压力和对象存储 spill 尚未验收。

产品合同见[产品定位](product-positioning.md)、[架构总览](architecture/overview.md)和[架构原则](../PRINCIPLES.md)。工程优先级见[Roadmap](../ROADMAP.md)与[实现任务](next.md)。
