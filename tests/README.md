# 基础框架验收

所有命令在 Linux 运行，入口见 [运行指南](../docs/foundation-running.md)。

- `error_contract.rs`：真实 TCP/UDS 错误身份一致、Meta 详情、REST JSON 与 FUSE errno。
- `config_contract.rs`：CLI/TOML 优先级、未知配置、编译/运行后端选择。
- `vfs_contract.rs`：单 mount/单 Backend 接入、文件/目录句柄接口的未实现边界与指标。
- `src/node/vfs/ownerfs/root.rs` 内单元测试：本机根授权激活后的准入、权限拒绝、Holder/Home 会话不匹配与失效封闭；不代表首次 mkdir 或重启恢复已接通。
- `storage_localfs.rs`：受限本地路径、打开一次后的句柄 I/O、短读写及显式同步。
- `fuse_contract.rs`：真实 FUSE 挂载、独立 Backend session 与已有挂载保护；需显式 `--ignored`。
- `local_sdk.rs`：真实 UDS + SHM，8 字节读写、无 SHM 拒绝、错误/取消与 socket 回收。
- `meta_contract.rs`：Meta Ping、OwnerRoots/DfsMeta 与 RecoverRoot 接口合同、请求/错误指标。
- `rdma_lifecycle.rs`：显式真实 RXE/RDMA 搬运、取消后会话拒绝复用、不重放、TTL 过期；需配置设备并显式 `--ignored`。
- `feature-matrix.sh`：OwnerFs/DFS 独立编译，Meta 零后端，公共传输无默认特性，SDK 不引入 RDMA。
- `scripts/dfs/r1_e2e.py`：真实 Meta、DFS Node 和独立 FUSE mount，验证 R=1 create/write/fsync/reopen/read 与 immutable Chunk。
- `ownerfs_acceptance.py`：OwnerFs 多节点功能和恢复验收。

传输错误与协议边界测试同时位于 `src/node/rpc`、`common/transport`；OwnerFiles 的远端文件协议目前只验证注册与明确拒绝，未实现实际文件访问。没有用 Ping 冒充文件业务、授权或恢复完成。Python 仅是验收工具，不是运行 AFS binary 的依赖。
