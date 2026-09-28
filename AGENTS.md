# AFS（Agent FS）协作规则

当前 `main` 是 AFS 主线；旧 DMS 内存 KV 实现保存在 `mem-kv` 分支。

## 必读顺序

1. [README](README.md)：产品入口和当前能力。
2. [架构原则](PRINCIPLES.md)：稳定产品合同。
3. [产品定位](docs/product-positioning.md)：适用范围、优势场景和非目标。
4. [架构总览](docs/architecture/overview.md)与[数据 Profile](docs/architecture/profiles.md)：模块边界和数据语义。
5. [实现状态](docs/status.md)与[实现任务](docs/next.md)：真实能力和工程入口。
6. 当前任务对应的 RFC、源码和验收证据。

`docs/requirements.html`、`docs/architecture.html` 和 `docs/workloads.md` 已标记为 Superseded，只用于追溯历史。

## 产品合同

- AFS 是面向业务集群近计算场景的通用 POSIX 分布式文件系统。
- DistributedFs（DFS）是通用分布式主线，以不可变 Chunk 和 FileVersion 统一承载普通可变文件与不可变优化负载。
- OwnerFs 是 1～4 节点一体机 Agent workspace 的专用后端，以 Home 本地普通文件和跨 Node P2P 访问获得局部最优路径。
- 不可变镜像、Snapshot、Checkpoint 是 DFS 的重点优化负载，不是 DFS 的全部范围。
- 本地 SSD、NVMe、HDD 共同构成集群内持久副本与多级缓存；外部对象存储是可选 spill 层，不是系统成立条件。
- Meta 处理 namespace、文件身份、布局、版本、placement 和授权，不代理稳态文件内容。
- `fsync` 提交持久 FileVersion；Snapshot/Pin/Publish 是显式业务操作，不从 `close` 或 `fsync` 推断业务发布。
- 多源读取必须固定同一 FileVersion，并只选择通过 Chunk 身份和摘要校验的持久副本、cache 或消费者种子。

## 事实、设计和能力声明

- 文档必须区分 **Implemented**、**Experimental**、**Accepted Design**、**Proposed** 与 **Superseded**。
- 新增路径、类型或接口不等于功能完成；性能目标不等于实测收益。
- 产品语义变更必须先提交 RFC，并同步原则、架构、状态和验收标准。
- 实现细节只写入对应模块文档或源码，不复制为新的产品合同。
- 能力声明必须附带源码、测试、实验或命令输出位置。

## 代码边界

- `afs-meta` 是控制面；`afs-node` 承载 FUSE、OwnerFs/DistributedFs、ChunkStore、P2P 和本机 SDK 接入。
- FUSE、Native SDK 和未来的 runtime integration 使用同一 namespace 与后端语义，性能路径不能绕过授权、版本或校验合同。
- DFS 的所有文件共享 namespace、Meta、FileVersion、LayoutRoot、ChunkStore、placement、transport 和运维体系；不可变负载通过 Pin、Alias、RootManifest 和读取策略优化，不建立第二套文件对象。
- OwnerFs 与 DistributedFs 是独立后端。共同 FUSE 入口只做 namespace 和请求分派，不把 Owner/Home 语义扩散到 DFS。
- 本机 client 只访问本机 Node。Node 到 Meta 使用控制 RPC；Node 间数据路径使用共同 API 下的 gRPC、SHM 或可选单边 RDMA adapter。
- `common/` 只保留已有两个真实调用方的公共能力，不为预期复用增加框架。
- 不恢复 NFS 产品后端，不从旧 KV 分支复制权威状态机。
- staging、verified cache、durable replica 和 external committed 必须使用[副本状态](docs/semantics/copy-states.md)中的明确术语。

## 验证与交接

- macOS 只用于阅读与编辑；Rust 构建、服务、测试和性能验证在 Linux VM 或容器运行。
- 代码变更至少检查格式、编译、Clippy 和受影响测试；纯文档变更检查链接、格式、术语一致性和 GitHub 模板语法。
- 性能报告包含环境、输入、对照语义、样本和 p50/p95/p99；故障结论必须有真实恢复证据。
- 每个阶段更新 `docs/status.md` 和 `docs/next.md`。重大设计进入 RFC，状态事件写入研究工作区记录。
- 提交遵守上层工作区 `AGENTS.md` 的 Lore commit 协议；不得提交凭据、本地运行数据或临时证据。
