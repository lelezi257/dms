# 当前状态

更新时间：2026-09-25。

**决策：** AFS 面向 Agent workspace 与不可变镜像/快照。近计算部署、P2P 数据面、一个 Worker 进程承载 FUSE 和两套隔离后端；Home 仅指根的数据所在节点。NFS 不在产品范围内。

**本轮设计修订：** [详细架构](architecture.html)新增统一抽象语义、进程/模块命名、根解析与授权/撤销 ACK、删除重建、运行时显式快照及发布、多点读时序和 RPC 预算。[需求合同](requirements.html)、宪法与 AGENTS 同步。Workspace 热路径基于已获得的根授权；镜像 close/fsync 不自动发布。完成三次设计复核，并修正 copy-up 额外远端读取、根删除授权屏障和副本暂存保护。

**控制面决策：** MetaStore 首版采用 etcd，后续可嵌入 Rust Raft；条件事务、权威读、持久提交、恢复和围栏属于统一后端合同。单节点持久与三节点多数派容错分别声明。镜像数据/manifest 的两个物理副本与 Master 共识是不同维度。上一版手写双份控制日志方案已替换。

**事实与依据：** 3FS 白盒未发现完整沙箱快照发布机制，普通 write 通常不访问 Meta，不能据此编造比较优势。RustFS 本次审计主线未找到 Raft 依赖依据；OpenRaft/raft-rs 是独立候选。代码位置和官方链接见架构正文。

**当前实现：** 基础分支来自 origin/main@7e210b4，仅保留通用观测与传输 crate，没有运行中的文件服务；本轮未改产品代码，未运行新的 Linux 功能/性能测试。HTML/SVG 结构与本地链接作静态校验；浏览器视觉预览受工具策略限制，未完成视觉验收。旧 Home 与镜像实验只作参考。

**待验证：** 根访问屏障和进程恢复、FUSE 缓存/文件身份、P2P 未知结果、runtime 稳定切点及 upper/base 还原、两副本发布/GC、MetaStore 故障、真实 gVisor/Firecracker 接入和 RPC/性能预算。见 [下一步](next.md)。
