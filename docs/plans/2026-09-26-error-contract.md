# 补齐 AFS 错误合同

依据 main 的 common/error 与 gRPC richer error 实现，恢复稳定 ErrorCode、ErrorKind、message 分层；不恢复旧 KV/Arena/Journal 业务。

1. 建立协议无关错误类型、唯一编号目录、未知编号保留和 I/O 分类。沿用 main 的组件/子系统/原因编码，旧编号不改作新语义。
2. Proto 定义 AFS ErrorDetail；gRPC 使用 google.rpc.Status + Any，统一编码与解码。缺少详情按原生 gRPC 类别兜底，损坏或不一致详情报告协议错误。
3. 接入当前 Meta、Node 控制/数据、SHM、SDK、Peer、REST、FUSE；本地与远端存储错误一致，不按 message 判断业务，不自动重试未知结果。
4. Linux 验证编号、未知码、错误详情、边界映射，以及实际 TCP/UDS 请求的错误往返；执行 fmt、workspace 测试、Clippy、feature 矩阵和进程 E2E。文档记录兼容边界。

范围：当前基础框架；完整文件业务尚未实现，不预造未来所有业务错误。暂不提交或推送本次后续修改。

2026-09-26 实施完成：编号目录、标准 richer error、现有协议边缘和 SDK/Peer 均已接入；代码复核发现的已知错误码分类伪造已修复并补测试。独立架构复核因模型接口不可用未完成。最新验证与边界见 `docs/status.md` 和 `docs/error-contract.md`；原始材料留在研究区 `evidence/afs-error-contract-20260926/`，不随源码提交。
