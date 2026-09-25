# 下一步：依修订设计展开 Linux 实施

唯一输入是[需求合同](requirements.html)与[详细架构](architecture.html)。先使用“抽象语义”章固定类型和模块职责，再拆可观察的实现步骤；不要恢复旧 KV/Block 模型、NFS 产品后端或单独 Home 进程。

1. 闭合 Master/MetaStore（先 etcd）、WorkspaceRoot 创建/恢复/删除和 RootGrant。验证根内 mkdir 不查中心；首次远端访问严格在原节点屏障 ACK 与授权提交之后。
2. 在一个 AFS Worker 内接通 FUSE 与 WorkspaceService 本地文件路径，再接 P2P。按保守缓存合同验证同一根的 A/B 共享、长短写、旧 FD、重建和断回复；两个 namespace 的身份/权限/缓存键隔离。
3. RuntimeAdapter 显式 RequestSnapshot：私有 draft、稳定切点、持续写下一时刻内容；验证 base/upper/whiteout 与 guest 在途 I/O，区分 cut-ready 与 published。
4. 接通后台打包、两物理副本/manifest、原子版本提交、暂存保护、reader pin 和 GC，再验证 gVisor 文件树与 Firecracker 完整本地私有磁盘的真实接入。
5. 按架构 RPC 账核对调用数与消息/字节，再跑 W1/W2/W3、发布、冷/热启动和 fan-out。copy-up 和物化 base 缺失内容单独计量；Native/薄 FUSE 是参考，W1 快 MooseFS 20% 为待验证工程目标。

内嵌 Raft 是后续 MetaStore 实现，不能只换选主库而遗漏持久状态机、迁移、旧主围栏和故障验证。设计文档不等于已实现；旧分支数字不作为当前验收，所有构建、测试和性能测量继续在 Linux。
