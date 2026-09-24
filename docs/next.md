# 下一步：按双工作负载设计进入实施穿刺

以[需求分析](requirements.html)和[详细架构](architecture.html)为唯一输入，把模块边界及 W0–W5 / I0–I5 展开成 Linux 端到端实施和验收清单。先闭合根归属与 Home 本地普通文件，再验证 P2P 共享缓存/未知结果；镜像侧先证明稳定切点、两故障域 Blob + 控制记录原子发布，再接 gVisor 文件树和 Firecracker 完整磁盘文件。每一步报告本分支的新证据，不能继承旧 Home 分支的功能和性能结论。

优先技术穿刺：A/B 交替写和重开读；Master/Home/FUSE 故障与围栏；目标文件系统 reflink/快照与暂停复制回退；两节点发布中途断点和损坏恢复；gVisor 与 Firecracker 的真实运行时接入。性能分别对照 Native、薄 FUSE、MooseFS、文件树/磁盘文件分发基线，并采真实 Agent Home 命中率。
