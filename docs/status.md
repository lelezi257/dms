# 当前状态

更新时间：2026-09-25。

**代码导读（事实）：** 已补充配置、进程启动、FUSE/VFS、SDK、gRPC/RDMA、Storage 和资源回收的中文注释；阅读顺序见 [目录架构第 6 节](code-layout.md#6-从入口读代码)。`transport/native` 是 libibverbs C 适配层，不是文件后端。

**正式基础框架已落地（事实）：** [目录架构](code-layout.md)记录当前布局和边界。根 `afs`、`afs-client` 与六个 common crate 已接入 workspace；`afs-meta` 和 `afs-node` 是可启动进程。当前能力包括 CLI/TOML 配置、运行时和编译期 `OwnerFs/BlobFs` 开关、日志/metrics/trace、Meta REST/gRPC ping、Node REST、Node→Meta ping、Node→Node control ping、Node→Node data 8 字节读写、Local SDK UDS gRPC + SHM、FUSE namespace 分派。FUSE create 明确返回 `ENOSYS`，不假装文件已创建。

**本步验证（事实）：** Linux ARM64 / Rust 1.95.0：workspace 测试 55 项通过；默认跳过的 6 项设备测试随后显式执行，真实 FUSE 3 项、真实 RXE RDMA 3 项全部通过。fmt、Clippy all-features/all-targets、编译 feature 矩阵、binary/example 构建通过。gRPC 与 RXE 两套独立进程 E2E 各 22 项通过，包含 SDK UDS+SHM、FUSE 分派、REST、跨服务 OTLP Trace、8 字节数据校验及退出/冲突清理。验证结果与复现入口见[阶段验收摘要](foundation-milestone.md)。Python 只用于测试脚本；产品运行不依赖 Python。RXE 不是硬件性能或跨机器验收。

**数据面边界（事实/决策）：** 本机 SDK 使用 UDS gRPC 传控制信息，每次操作通过 sealed-size memfd + SCM_RIGHTS FD passing 传递共享缓冲，服务端以 pread/pwrite 拷贝字节；当前不宣称零拷贝。SDK 无 gRPC 内容 fallback；没有 SHM 的普通调用者走 POSIX。Node→Node 数据使用同一业务 API 下的 gRPC inline 或 RDMA adapter；RDMA 模式仍用 gRPC 下命令，内容走单边 READ/WRITE。RDMA 会话使用 `NegotiateData` 交换描述符和版本，再用真实 RDMA `SEND_WITH_IMM` 探测证明通道就绪；没有独立 `ReadyData` RPC。自动 fallback 只发生在建连阶段，强制 RDMA 不会降级。

**当前不是完整文件系统（事实）：** OwnerFs/BlobFs 业务状态机、根授权、共享屏障、真实 inode/dentry、持久恢复、镜像 draft/snapshot/publish、GC、生产鉴权和性能目标尚未实现。本轮 storage/diagnostics 是独立诊断对象，不是公开文件接口，也不代表逐写掉电持久合同。

**最新讨论与专项（优先于旧命名）：** 进程确定为 `afs-meta/afs-node`，统一 VFS 下为 `OwnerFs/BlobFs`；高性能 SDK 只访问本机，UDS 控制与共享内存数据分离。RPC adapter/Handler 与业务调度分层，公共 gRPC 只保留 config/security，不强制 actor；业务状态用 Tokio 任务和作用域锁按模块管理。旧 HTML 命名与旧分支模块不恢复为现行权威。

**决策：** AFS 面向 Agent workspace 与不可变镜像/快照。近计算部署、P2P 数据面、一个 Node 进程承载 FUSE/SDK/REST/P2P 和两套隔离后端；Home 仅指根的数据所在节点。NFS 不在产品范围内。

**待验证：** 根位置与授权、OwnerFs 本地普通文件热路径、跨节点共享撤销 ACK、FUSE 缓存/文件身份、进程崩溃恢复、P2P 未知结果、BlobFs runtime 稳定切点与显式发布、两副本发布/GC、MetaStore 故障、真实 gVisor/Firecracker 接入、真实 RDMA 硬件与性能预算。见 [下一步](next.md)。

**2026-09-25 RDMA 探测握手（事实）：** 参考 3FS 的 `SEND_WITH_IMM` connect probe，服务端在回复 `NegotiateData` 前准备接收槽，客户端连接 QP 后发送零字节探测并等待本地 CQ；服务端在首条数据请求前消费接收 CQ，成功后才允许文件访问和单边 READ/WRITE。探测使用固定立即数、独立 WR ID、状态/opcode/flags/byte_len 校验；超时或错误 poison 会话。握手版本为 1，两端需同时升级。最新 Linux 验证：57 项常规测试、3 项底层 RXE 探测、4 项节点 RXE 会话测试通过；gRPC/RDMA 两套独立进程 E2E 各 22 项通过，另有 fmt、严格 Clippy/Rustdoc、编译开关矩阵通过。见[阶段验收摘要](foundation-milestone.md)。RXE 仍不代表硬件性能结论。
