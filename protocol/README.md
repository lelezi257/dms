# DMS 进程协议

本目录维护跨语言通信约定，不是独立服务，也不是 SDK 用户直接编程的接口。用户入口见 [Rust SDK](../docs/rust-sdk.md)。

| 协议文件 | 通信双方与职责 |
| --- | --- |
| [client_node.proto](proto/dms/v1/client_node.proto) | SDK → Node：KV/KKV/range、会话、Staging 和 Region 授权；WorkerPayloadService 负责 gRPC 数据上传/下载。 |
| [node_peer.proto](proto/dms/v1/node_peer.proto) | Node ↔ Node：探测、拉取 Block、副本 prepare/activate/abort/status。每个 Node 可以同时作为调用方和服务方。 |
| [node_meta.proto](proto/dms/v1/node_meta.proto) | Node → Meta：会话/心跳、版本与位置解析、提交、副本上报、Watch 和 ACK。 |
| [types.proto](proto/dms/v1/types.proto) | 上述协议共享的数据结构；不包含业务处理逻辑。 |

## 生成与实现放在哪里

[build.rs](build.rs) 调用 protobuf/gRPC 生成器，生成 Rust Client/Server 类型；业务 Handler 仍在 [Node](../server/src/node/README.md) 和 [Meta](../server/src/meta/README.md)。同一 Service 可以有普通请求和 Stream，具体以 `.proto` 方法定义为准，不再维护第二份接口列表。

共享内存已实现：控制请求走 gRPC over UDS，FD 经独立 Unix Socket 的 `SCM_RIGHTS` 传递；用户 bytes 由共享 mmap 访问，而不是塞进 protobuf。相关安全边界见 [SHM](../common/shm/README.md)。RDMA/UB 尚未实现。

## 构建与发布边界

当前源码构建需要 `protoc`，生成文件位于 Cargo 构建输出中，不手工复制成另一份源码。预期安装用户无需 `protoc`、SDK 不向用户暴露生成类型；但独立 SDK 真构包和安装仍待发布阶段验收，不能从当前源码构建成功推导为已经满足。
