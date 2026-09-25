# AFS（Agent FS）：面向 Agent 工作负载的文件服务

这是从 main 建立的**新方向基础分支**，当前只保留可复用的日志、指标、追踪和传输组件；还没有可运行的 FUSE daemon、Master 或文件后端。旧内存 KV/对象 SDK 不属于本分支。已发布的旧版本与历史证据仍可从 Git 历史查看，不能把它们当成本分支的实现状态。

架构以近计算与 P2P 为特征：Worker 与计算节点共置，本地优先，跨节点直接访问和分发，Master 提供位置与权威管理。目标覆盖两类用途：Agent workspace 的亲和本地读写及必要的跨节点共享；以及私有写入、原子发布、发布后只读且可多点分发的镜像与快照。gVisor 的文件树和 Firecracker 的磁盘镜像文件都在镜像范围内。首版 Firecracker 可先完整下载镜像再启动，不要求块级懒加载。

先读 [宪法](PRINCIPLES.md)、[需求分析](docs/requirements.html)、[详细架构](docs/architecture.html)、[状态](docs/status.md)与[下一步](docs/next.md)。开发协作规则在 [AGENTS.md](AGENTS.md)。架构设计已经落盘，代码尚未实现；不能从保留的通用 crate 推断未来协议或进程能力。

目前 common/ 是唯一保留的 Rust workspace 代码：logging、metrics、tracing、transport。权威开发与构建环境是 Linux；工具链由 [rust-toolchain.toml](rust-toolchain.toml) 固定。
