# AFS 错误合同

沿用 main 的错误码策略，适配当前 AFS 模块。业务判断读取 `code()` 或 `kind()`，禁止解析 `message()`；错误分类不构成自动重试承诺。

## 三层含义

| 字段 | 用途 | 示例 |
|---|---|---|
| ErrorKind | 通用失败类别 | NotFound、PermissionDenied、DeadlineExceeded |
| ErrorCode | 精确且稳定的机器身份 | NODE_STORAGE_NOT_FOUND = 0x02070002 |
| message | 本次失败的诊断描述 | 文件不存在的具体原因 |

`common/error` 不依赖 Tonic、Prost、Axum 或 libc。`Error::coded(CODE, message)` 使用目录默认分类；`Error::new(code, kind, message)` 用于还原远端已知或未来错误。编码保持 `0xCCSSRRRR`：组件 / 子系统 / 原因；01 Client、02 Node、03 Meta、04 公共运行时。完整目录在根 [error-codes.toml](../error-codes.toml)，Rust 常量在 `common/error/src/code.rs`，测试校验名称、唯一编号和类别一致。

已复用 main 的 Client、Transfer、Meta 参数错误编号。旧 KV、Arena、Journal 等编号不重新分配；新 Storage、VFS、RDMA session、SHM 使用新子系统段。新增错误先登记常量与目录，再接入实际产生错误的调用点。

## 一次实际失败怎么返回

B 读取 A 的不存在文件：

1. A 的 Storage 得到文件不存在，统一转成 `NODE_STORAGE_NOT_FOUND + NotFound`。
2. A 的 RPC 边缘将其放进 `afs.error.v1.ErrorDetail`，再装入标准 `google.rpc.Status.details` 的 Any；外层 gRPC 状态为 NotFound。
3. B 的 gRPC 或 RDMA adapter 解码出相同 code/kind/message。RDMA 的文件命令/结果本来就走 gRPC，错误不另造 RDMA 协议。
4. 本机 SDK 经 UDS 读相同文件也得到同一身份；不会把 RPC 错误替换为 broker 收尾错误。

Storage 的转换只定义一次。权限不足、磁盘容量、目录类型等 I/O 保留可用分类，不随本地/远端入口改变。SHM 授权失败与参数错误分开。RDMA 现有失败后禁用会话、取消资源归属及写入不重放的机制不变。

## 协议边界

- **gRPC：** `common/transport/src/grpc/error_status.rs` 负责标准 envelope 和 AFS detail 编解码，供 SDK、Meta、Node 复用；不提供业务 Handler、重试或 actor。
- **SDK/Peer：** `code()` / `kind()` 暴露机器身份。SDK 的 `LocalClientError::Status` 现在承载原生 AFS Error，依赖旧 tonic Status 枚举载荷的调用代码需要调整。
- **REST：** `src/error.rs` 输出对应 HTTP 状态与 `{"error":{"code":数值,"kind":"类别","message":"诊断"}}`。先前纯文本错误体变为结构化 JSON；成功响应不变。
- **FUSE：** POSIX 只能返回 errno。映射留在进程层，同一错误的数字码、分类和消息写入日志；这只是当前已实现操作的映射，不声称覆盖全部未来 POSIX 特殊错误。
- **OwnerFs 重启：** `NODE_OWNER_STALE_ACCESS` 与 `NODE_OWNER_STALE_HANDLE` 在 FUSE 边缘映射为 `ESTALE`；前者要求重新获取根授权，后者要求重新 lookup/open。RPC 服务端尚未实现发出这两种业务错误，现阶段仅固定错误身份与边缘映射。
- **启动/底层设施：** 操作系统、TLS/配置库等内部机制仍可持有自身错误类型；当前业务 RPC 的失败出口使用统一编解码。进程初始化错误不伪装成远端业务码。

## 混合版本与未知错误

- 新服务端使用 AFS v1 type URL，旧客户端仍能读取标准 gRPC code/message，但不能得到精确 AFS 码。
- 新客户端接到旧节点或 gRPC 框架的无详情错误，用 `CLIENT_REMOTE_STATUS` 保留 gRPC 分类。它不猜测远端的具体业务码。
- 未知数字码原样保留；未知码附带未来未知 kind 时降为 Unknown，数字码不丢失。已知码必须匹配目录分类；`CLIENT_REMOTE_STATUS` 是保留原生 gRPC 分类的明确例外，可被中间节点转发。
- 损坏 envelope、重复 AFS detail、零编号、内外状态/消息或 kind 与 gRPC 类别冲突，返回 `CLIENT_PROTOCOL_INVALID_ERROR_DETAIL`。
- 不兼容旧 DMS type URL 或裸 ErrorDetail；两个产品协议不相同。没有必要在新 AFS 内复制旧 DMS 早期兼容分支。
- DeadlineExceeded、Unavailable 只能说明等待或通道失败，不能证明写未发生。没有错误触发的自动 fallback/replay。

验证入口：`common/error` 目录校验、`error_status` 往返/畸形详情测试、`tests/error_contract.rs` 的真实 TCP/UDS 和边缘映射测试，以及既有 SDK/RDMA/E2E 用例。
