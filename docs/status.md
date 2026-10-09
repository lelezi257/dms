# 当前状态

当前状态以 [当前计划](development/plan.md) 为准。本页只保留便于旧链接跳转的简短摘要，不再追加逐轮日志。

- G1 历史试用范围保持 8/8 关闭。
- 当前可运行交付是 a103 OwnerFs workspace bind ON 有限试用包。
- G2 主线优先级是 bind ON 当前场景、远端访问协同、DFS 一写多读，再到普通本地 FUSE。
- 普通 OwnerFs 读写性能目标为吞吐 `>=1.2x` 同条件 MooseFS，独立操作时延 `<=0.8x` 同条件 MooseFS；当前尚未达标。
- DFS 性能目标为三同步持久副本条件下持平 3FS；当前待验。
- 当前源码已迁移固定官方 `fuser =0.18.0`，无私有 vendor；Linux 限定权限、mmap、正常卸载及 bind ON＋远端核心场景通过。跨节点 fcntl/flock、等待取消、bind/native↔FUSE 锁域协同后置；不继承旧候选完整验收，详见 [依赖决策](development/fuser-official-blocker.md)。

详细任务、出口和后置项见 [当前计划](development/plan.md)。
