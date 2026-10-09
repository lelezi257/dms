# 路线图

当前路线按可独立验收的小项推进，先保证可运行、可试用，再逐步补性能和复杂可靠性。详细任务 ID 和状态以 [当前计划](docs/development/plan.md) 为准。

| 能力 | 依赖 | 验收出口 |
| --- | --- | --- |
| 可试用版本 | OwnerFs/DFS 基本挂载、local-file Meta、安装包 | 同事可独立安装、启动、写入、停止、重启并读回限定数据 |
| OwnerFs workspace bind ON | Home 身份、授权、bind mount、卸载排空 | 真实 Home 底层目录覆盖 OwnerFs 一级 workspace，权限和远端可见性正确 |
| OwnerFs 远端读写 | Home 句柄、peer 传输、close-to-open、错误传播 | 吞吐与独立时延分别达到 MooseFS 双目标，未达标时保留数据继续优化 |
| DFS 一写多读 | chunk、版本、三同步持久副本、读计划 | 一写确认后多读者内容一致，性能与 3FS 同条件对照 |
| 普通 OwnerFs 本地 FUSE | FUSE 热路径、缓存策略、屏障语义 | 保留 MooseFS 双目标，但优先级低于 bind 和远端场景 |
| 官方 fuser 迁移 | 固定官方 `=0.18.0`、自有适配层；限定迁移验收完成 | 解除私有补丁，保持新鲜度、close-to-open、权限、错误传播、持久化、direct-I/O mmap 协商及正常卸载/排空；构建通过不等于功能验收 |
| 分布式锁专题 | 保留后端锁实现/测试，按统一 E2E 行为清单评估官方 fuser、fuse3、fuse-backend-rs，最后自研 | 跨节点 fcntl/flock、冲突/等待/取消/释放及进程/FD 生命周期、bind/native↔FUSE 锁域协同后置，不作为本次迁移前置 |
| 复杂可靠性 | 多 Meta、故障注入、长时运行、etcd/Redis | 后置专题，不阻塞当前迁移前快照 |

本轮迁移前快照不执行 Agent DX 正式导入，只形成清洁源码、中文正式文档、可追溯试用包和明确阻塞项。

内核单挂载本地回退锁与 bind 路径的本机 ext4 锁属于不同锁域，不能作为分布式锁通过证据；本次必要正确性和依赖验收见 [决策说明](docs/development/fuser-official-blocker.md)。
