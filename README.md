# AFS（Agent FS）：面向 Agent 工作负载的文件服务

这是从 main 建立的**新方向基础分支**。本分支已经把正式底座拉起来：`afs-meta` 与 `afs-node` 可启动，支持 CLI/TOML 配置、REST/gRPC ping、Node→Meta、Node→Node control/data、Local SDK UDS + SHM、FUSE namespace 分派、日志/指标/Trace 和 8 字节诊断读写。它仍然不是完整文件系统：FUSE create 会明确返回 `ENOSYS`，OwnerFs/BlobFs 还没有真实 workspace 或镜像发布业务。

架构以近计算与 P2P 为特征：afs-node 与计算节点共置，本地优先、跨节点直连，afs-meta 提供位置与权威管理。OwnerFs 面向 Agent workspace，BlobFs 面向私有写入、显式发布和不可变多读的镜像/快照。gVisor 文件树与 Firecracker 磁盘镜像都在范围内；首版 Firecracker 可先完整拉取镜像，不要求块级懒加载。

先读 [宪法](PRINCIPLES.md)、[需求分析](docs/requirements.html)、[详细架构](docs/architecture.html)、[目录架构](docs/code-layout.md)、[运行指南](docs/foundation-running.md)、[状态](docs/status.md)与[下一步](docs/next.md)。开发规则在 [AGENTS.md](AGENTS.md)。目录文档承接已确认的 Meta/Node、OwnerFs/BlobFs 命名与接口分界，不从旧 HTML 的角色名称恢复旧模块布局。

workspace 包含根 `afs`、`client/` 与六个 `common/` crate。`common/transport` 提供 gRPC config/security、SHM FD passing 与可选 RDMA native shim；公共 gRPC 只封装配置/安全，不封装业务 Handler 或 actor。权威构建与测试环境为 Linux，工具链由 [rust-toolchain.toml](rust-toolchain.toml) 固定。Python 只用于仓库 E2E 脚本，不是产品运行依赖。
