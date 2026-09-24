# 当前状态

更新时间：2026-09-25。

**已决：** 产品范围为 Agent workspace 与发布后不可变的镜像/快照；采用 Master/worker 和共同 FUSE 入口、两套业务后端。gVisor 与 Firecracker 均在目标范围内。

**名称：** 本分支产品名为 AFS（Agent FS）；GitHub 仓库和当前分支名暂不改，旧 DMS 历史仍保留在原分支。

**当前实现：** 本分支从 origin/main 的 7e210b4 建立，旧 KV 服务、协议、SDK、旧文件系统、历史验收与发布脚本已清出。仅保留并裁剪通用观测和连接组件。没有运行中的文件服务，没有任何这两类工作负载的功能或性能验收。

**设计已完成、实现未开始：** [需求分析](requirements.html)列出 W0–W5 / I0–I5、首版操作覆盖和验收口径；[详细架构](architecture.html)确定单 FUSE 入口、独立双后端、粗粒度 Master、Home 本地文件/P2P-NFS、镜像稳定切点/不可变 Blob/两故障域副本与控制记录。两份 HTML 完成三轮自审；它们是设计决策，不是当前代码能力或 Linux 性能结果。

**待验证：** 共享模式缓存切换、P2P 非幂等请求的结果未知处理、稳定切点、双故障域控制记录恢复、gVisor/Firecracker 真接入、真实 Agent Home 命中率、两类工作负载的 Linux 功能/故障/性能矩阵。见 [下一步](next.md)。
