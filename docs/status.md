# 当前状态

当前状态以 [当前计划](development/plan.md) 为准。本页只保留便于旧链接跳转的简短摘要，不再追加逐轮日志。

- G1 历史试用范围保持 8/8 关闭。
- 当前可运行交付是 a103 OwnerFs workspace bind ON 有限试用包。
- G2 主线优先级是 bind ON 当前场景、远端访问协同、DFS 一写多读，再到普通本地 FUSE。
- 普通 OwnerFs 读写性能目标为吞吐 `>=1.2x` 同条件 MooseFS，独立操作时延 `<=0.8x` 同条件 MooseFS；当前尚未达标。
- DFS 性能目标为三同步持久副本条件下持平 3FS；当前待验。
- 官方 `fuser` 迁移因公开 API 缺少 lock flags 与 interrupt 回调而阻塞；当前保留 vendored 版本以维持可编译可运行，详见 [阻塞说明](development/fuser-official-blocker.md)。

详细任务、出口和后置项见 [当前计划](development/plan.md)。
