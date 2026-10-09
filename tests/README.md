# 测试入口

测试以 Linux 为准运行。涉及 FUSE、挂载、权限、runc、RDMA、MooseFS 或 3FS 的用例需要对应 VM、权限和显式 case 选择；macOS 只用于编辑、静态检查和整理输入。

## Rust 合约测试

- `config_contract.rs`：CLI、TOML 优先级、未知字段和后端 feature 选择。
- `error_contract.rs`：TCP、UDS、REST、FUSE 边界上的结构化错误。
- `fuse_contract.rs`：FUSE session 与后端契约；需要权限的用例保持 ignored 或显式选择。
- `local_sdk.rs`：UDS 和 SHM 方向的本地 SDK 行为。
- `meta_contract.rs`：Meta ping、OwnerRoots、DFS Meta 与恢复契约。
- `ownerfs_peer_contract.rs`：OwnerFs peer 协议边界。
- `rdma_lifecycle.rs`：具备 RDMA 环境时的生命周期检查。
- `storage_localfs.rs`：local-file 存储安全和范围 I/O。
- `vfs_contract.rs`：后端 trait 与 VFS 边界。

## 验收工具

- [`tests/acceptance/`](acceptance/)：OwnerFs、DFS、环境准入、标准套件、workspace bind 和打包安装相关的验收驱动。这里保留维护中的 runner、drivers、probes、tests 和必要小 fixture；历史过程证据已迁出源码树。
- `tests/feature-matrix.sh`：OwnerFs、DFS、zero-backend Meta 和 transport crates 的 feature 组合检查。
- `scripts/dfs/r1_e2e.py`：启动真实 Meta 和 DFS Node，挂载 FUSE，写入、同步并读取文件。
- `scripts/ownerfs/accept_three_vm.py`：OwnerFs 多节点验收入口。

## 范围说明

测试只证明其覆盖的行为。当前测试集不代表生产 HA、完整 POSIX、完整 R=N 复制、VerifiedCache、SeedLease、Spill 或所有 crash-recovery 场景已经完成。阶段、优先级和验收出口以 [`docs/development/plan.md`](../docs/development/plan.md) 为准。
