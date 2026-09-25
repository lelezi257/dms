# 基础框架验收

所有命令在 Linux 运行，入口见 [运行指南](../docs/foundation-running.md)。

- `config_contract.rs`：CLI/TOML 优先级、未知配置、编译/运行后端选择。
- `vfs_contract.rs`：namespace 分派、关闭后端、Unsupported 错误与指标。
- `fuse_contract.rs`：真实 FUSE 挂载、两个后端回调、关闭 namespace、已有挂载保护；需显式 `--ignored`。
- `local_sdk.rs`：真实 UDS + SHM，8 字节读写、无 SHM 拒绝、错误/取消与 socket 回收。
- `meta_contract.rs`：Meta Ping 与请求/错误指标。
- `rdma_lifecycle.rs`：显式真实 RXE/RDMA 搬运、取消后会话拒绝复用、不重放、TTL 过期；需配置设备并显式 `--ignored`。
- `feature-matrix.sh`：OwnerFs/BlobFs 独立编译，Meta 零后端，公共传输无默认特性，SDK 不引入 RDMA。
- `foundation_e2e.py`：真实 Meta、两个 Node、独立 SDK 调用者和 OTLP collector。校验数据、FUSE 错误分派、进程退出、资源冲突、实际跨服务 Trace；支持 gRPC 与真实 RXE/RDMA。

传输错误与协议边界测试同时位于 `src/node/rpc`、`common/transport`；没有用 Ping 冒充文件业务、授权或恢复完成。Python 仅是验收工具，不是运行 AFS binary 的依赖。
