# 当前状态

更新时间：2026-09-27。AFS 已成为 `main` 主线；旧 DMS 内存 KV 实现保存在 `mem-kv` 分支。当前只交付源码与文档，尚未发布 AFS 版本。

**已实现（事实）：** AFS 基础框架含 CLI/TOML、OwnerFs/BlobFs 编译与运行开关、FUSE/SDK/REST 入口、gRPC/RDMA 数据通道、日志/metrics/trace/error。OwnerFs 使用本机普通文件和 Node 间 P2P；Meta 负责节点注册与粗粒度根授权，不进入每次文件 I/O。MetaStore 统一提交队列在后端 ACK 后发布可见状态，当前后端为默认 `etcd`、显式 `local-file` 和 `memory`。BlobFs 仍是骨架。架构与边界见[目录架构](code-layout.md)、[MetaStore 提交合同](plans/2026-09-27-meta-store.md)。

**本阶段验收（事实）：** 最终 Linux Release 二进制在三 VM 功能 15/15；带 `fh` 的属性操作与同句柄 I/O 保序，Home 对打开句柄校验实际根、peer 与授权，异步 RELEASE 短暂失败有界重试；B daemon 持有远端 FD 后被 `SIGKILL`，Home 后台记录回收 1 个遗留句柄。全特性测试、严格 Clippy、格式、release 构建通过。详见[修复与复验](reviews/2026-09-27-ownerfs-p2p-hardening.md)。

**性能（事实）：** 同一最终二进制的 W1 本机私有根两份 6 轮为 MooseFS 的 **0.482/0.451**；200×4 KiB、8 worker 完整 W2 两份 12 轮为 **0.768/0.780**；顺序 W2 两份 6 轮为 **1.106/1.080**。W1 同场本机 Native FS 147/144 ms、薄 FUSE 234/235 ms、OwnerFs 342/316 ms、MooseFS 709/701 ms。固定并发 W2 达到 0.8 目标，但顺序 W2 未证明稳定达到 1.10；不把此固定负载推广到所有工作负载。原始运行数据留在研究工作区 `experiments/results/2026-09-27-afs-p2p-hardening/`，不纳入 Git。

**未完成（事实）：** Meta 单活动围栏/选主、根删除及跨节点根列举、常用 `chmod/chown/atime/mtime`、完整 POSIX、VM 掉电和长稳尚未验收。`memory` Store 易失、`local-file` 只承诺单机/单盘，Redis 未实现。文件内容仍走 gRPC P2P，RDMA 文件内容路径未接通；不能宣称完整 MooseFS 替代。下一入口见[下一步](next.md)，前阶段审视留在[评审记录](reviews/)。
