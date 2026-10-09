# 产品定位

AFS 是面向 Agent、Sandbox 和 VM 集群的近计算文件系统。它暴露普通路径和 POSIX 风格操作，把计算节点本地磁盘作为热数据、持久副本、验证缓存和节点间读取的主要资源池。

AFS 当前有两条后端路径：

| 后端 | 适用范围 | 主要价值 |
| --- | --- | --- |
| `OwnerFs` | 小规模 Agent workspace，通常 1 到 4 个节点 | Home 节点保存普通本地文件，计算远离 Home 时通过 peer 转发访问 |
| `DistributedFs` (`DFS`) | 更通用的共享分布式文件系统 | 可变文件基于不可变 chunk 提交，支持副本、版本一致读、P2P、缓存和可选 spill |

当前实际使用场景优先开启 **OwnerFs workspace bind mount**。第一阶段目标不是把所有后端都打磨完，而是先交付一个可编译、可运行、可试用的版本：bind ON 场景功能闭环，远程访问正确，核心性能逐步达标。

## 适合的工作负载

- Agent workspace 在 Home 上频繁小文件读写，同时需要被远端计算节点访问。
- 需要普通写、sync、reopen 和 close-to-open 可见性的共享可变文件。
- 镜像、快照、checkpoint 和数据集在稳定后成为 DFS 固定版本。
- MicroVM 基础镜像固定读取，写层产生新的 DFS chunk。

## 边界

- AFS 不承诺所有 POSIX 工作负载都比本地文件系统快。
- `close` 成功表示此前写入完成必要 flush 并对后续 open 可见；文件 sync 不等于父目录 sync。
- `fsync` 不创建业务发布、别名、pin 或快照。
- 验证缓存只有被提升并提交后才算持久副本。
- 外部对象存储是可选冷容量，不是当前主线的强依赖。

## 当前优先级

1. OwnerFs workspace bind mount：bind ON 可用、远端协同正确、试用包可交付。
2. OwnerFs 远端普通 FUSE 读写：吞吐和时延分别对比 MooseFS 达标。
3. DFS 一写多读核心场景：与 3FS 同条件对照。
4. 普通 OwnerFs 本地 FUSE 优化：保留目标但后置。
5. 大规模、长时间、复杂可靠性、多 Meta、etcd 和 Redis 后置。

完整任务 ID、状态和判据见 [当前计划](development/plan.md)。验收规则见 [验收说明](testing/acceptance.md)。
