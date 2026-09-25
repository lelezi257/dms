# AFS（Agent FS）协作规则

本分支从 origin/main 建立，服务 Agent workspace 与不可变镜像/快照两类工作负载。先读 [宪法](PRINCIPLES.md)，再读 [需求分析](docs/requirements.html)、[详细架构](docs/architecture.html)、[目录架构](docs/code-layout.md)、[状态](docs/status.md)和 [下一步](docs/next.md)。业务语义以原则、需求与详细架构为准；已确认的目录/模块命名和依赖分界以 code-layout.md 为准，不从旧 KV 代码或旧分支推断。

## 事实与决策

- 区分**已决方向**、**当前实现**、**推断**与**待验证**。新增路径或类型不等于功能已完成；性能目标不等于实测收益。
- 改变两类工作负载的写入、发布、读取、故障或副本语义时，先更新原则和相应设计，并说明对验收的影响。普通实现细节不复制进本文件。
- docs/workloads.md 只写简要用户可见语义，docs/requirements.html 是正式需求合同，docs/architecture.html 是正式架构正文；同一机制不另建并行权威。docs/status.md 记录当前已验证能力，docs/next.md 只保留下一阶段入口。历史决策留在 Git 历史，不把已删除旧文档重新标为现行规则。

## 术语与时序约束

- 业务抽象参见 docs/architecture.html，目录/模块名见 docs/code-layout.md。进程为 afs-meta/afs-node，后端为 OwnerFs/BlobFs。DataHome 是位置，不是进程；FUSE 与本地文件/P2P 服务在同一个 afs-node 中。旧 HTML 的 Meta/Node 是角色描述，不另建同名模块。
- 根位置查询不是访问授权；首次跨节点访问先完成撤销屏障和原节点 ACK，再提交新授权。本地已授权热路径不逐操作找 Meta。
- Workspace 与 Image 为隔离 namespace；同一 workspace 跨节点访问仍需缓存一致性。镜像由 runtime 显式触发快照，不从 close/fsync 推断发布。
- 控制面先 etcd，后可内嵌 Rust Raft；MetaStore 提供条件事务和可恢复提交，不能把普通 Redis、选主锁或 watch 通知当作同等保证。

## 代码边界

- 以近计算与 P2P 为数据面原则：本地直接访问本机文件，跨节点 node 直连；不恢复 NFS 产品后端。Meta 只处理控制面，不代理数据内容。
- 业务后端按 workspace 与 immutable image 分开。共同 FUSE 入口只负责命名空间和请求分派；Meta 只负责粗粒度元数据。不得把旧 KV 的 Object、Block、Extent、Current 或逐写 Meta commit 当成新架构的默认前提。
- common/ 只保留确实可跨两个后端使用的观测和传输工具。抽取前先指出两个真实调用方，不为想象中的通用性增加框架。
- 本机 client/ 只访问本机 Node：UDS gRPC 控制与 SHM 数据，无 SHM 时走 POSIX。Node→Meta 与 Node 控制用 gRPC，Node 数据用共同 API 下的 gRPC/单边 RDMA adapter，文件命令两种模式共用 Proto。公共 gRPC 仅 config/security，不封装业务 client/Handler/actor。
- 不在 workspace 本地主路径强制镜像转换或远端发布；不向读者暴露未发布镜像。跨后端操作必须有明确结果，不能隐式改变数据语义。
- 本分支删除旧产品实现和旧 FUSE 补丁，需用到时从 Git 历史或旧分支按新合同重新引入；不要整目录复制旧权威状态机。

## 验证与交接

- macOS 仅用于阅读与编辑；Rust 构建、测试和性能验证在 Linux VM/容器。保留锁定工具链和 Cargo.lock，变更后至少检查格式、编译、Clippy 和受影响测试。
- 设计与实现分别验收。性能报告写明环境、输入、对照语义、样本和 p50/p95/p99；故障结论要有真实恢复证据。不能把过去分支的数字写成当前结果。
- 每次阶段结束更新 docs/status.md 和 docs/next.md。提交使用上层工作区 AGENTS.md 的 Lore commit 协议。不要把本地运行数据、凭据和临时证据加入 Git。
