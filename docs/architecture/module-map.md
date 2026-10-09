# 模块地图

本页只给新接手者提供当前源码入口。真实行为以代码和测试为准。

| 区域 | 入口 | 说明 |
| --- | --- | --- |
| Meta 服务 | `src/bin/afs-meta.rs`、`src/meta/` | 命名空间、inode、版本、租约和后端持久化 |
| Node 服务 | `src/bin/afs-node.rs`、`src/node/` | FUSE、OwnerFs、DFS、peer、chunk 和生命周期 |
| OwnerFs | `src/node/vfs/ownerfs/` | Home 本地文件、远端访问和 workspace bind |
| OwnerFs bind | `src/node/vfs/ownerfs/bind_mount.rs` | 核心挂载、身份核验和卸载，不承载 runc 业务 |
| DFS | `src/node/vfs/dfs.rs`、`src/node/vfs/dfs/`、`src/node/dfs_read.rs` | dirty 状态与读写路径 |
| 存储与副本 | `src/node/storage.rs`、`src/node/storage/localfs.rs`、`src/node/replication.rs` | chunk、本地存储与副本 |
| 部署脚本 | `scripts/deploy/` | 打包、安装和进程管理 |
| 验收工具 | `tests/acceptance/` | 维护中的验收工具、探针和固定 fixtures |

过程日志和历史原始证据不在产品源码树中维护，迁移前本地快照见仓库上一级 `local-archive/`。
