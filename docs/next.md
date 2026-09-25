# 下一步

当前基础框架已完成到可运行、可验证的 hello/ping/8 bytes 底座。运行方式见 [foundation-running.md](foundation-running.md)，当前能力与限制见 [status.md](status.md)。

代码阅读入口见 [目录架构第 6 节](code-layout.md#6-从入口读代码)，各层已补中文职责与时序注释。

下一阶段沿已确认架构实现真实业务，建议顺序：

1. **Meta 根目录登记与位置查询。** 实现 WorkspaceRoot 创建、Home 归属、节点注册、调度可查询的位置接口；遵守已确认的 etcd-first MetaStore 条件事务与持久提交合同，后续可替换为内嵌 Raft；不另造简化持久权威。
2. **OwnerFs 本地最小工作流。** 在本机授权根下实现普通文件/目录的 create/open/read/write/rename/unlink 最小集合，热路径只走本地文件和 FUSE/VFS，不转 Blob，不逐操作提交 Meta。
3. **OwnerFs 跨节点共享入口。** 实现远端访问前的撤销屏障、原节点 ACK、授权切换和 P2P 读写；用 A/B 交替写、不同长度、删除重建、旧句柄、进程故障验证 close-to-open 与恢复边界。
4. **BlobFs 私有写入与显式发布。** 明确 runtime API 触发 snapshot/publish，不从 close/fsync 推断发布；先打通一个 draft 到 immutable version 的完整链路，再做多点读和副本策略。
5. **性能穿刺。** OwnerFs 对比 thin FUSE/MooseFS，重点看本地热路径是否接近 thin FUSE 并稳定快于 MooseFS；BlobFs 对比镜像懒加载/多读场景，单独报告缓存命中、冷启动和并发启动。

本轮的 Storage 诊断对象不带业务授权，不能直接扩展成公开可写文件接口。SHM 与 RDMA 传输完成也不等于持久化/发布完成。下一阶段不重复讨论已确认的进程、目录与通道选型。

RDMA 会话已改为一次 `NegotiateData` 加真实通道探测，后续性能分析分别计入建连探测、每次文件命令的 gRPC 和内容单边搬运成本；当前暂存缓冲区仍有拷贝，零拷贝与真实硬件性能尚未验收。
