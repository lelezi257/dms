# AFS

AFS 是面向 Agent、Sandbox 和 VM 集群的近计算文件系统原型。当前源码可在 Linux ARM64 上编译，已发布一个有限但可运行的 OwnerFs workspace bind ON 试用包：[afs-bind-a103a2f](https://github.com/lelezi257/dms/releases/tag/afs-bind-a103a2f)。

这个快照的目标是作为迁移到 Agent DX 前的清洁基线：产品源码、维护中的测试和正式文档在仓库内；逐轮证据、checkpoint、历史实验和过程计划已归档到仓库上一级 `local-archive/`，不作为产品树内容。

## 当前能力

- `afs-meta` 管理 namespace、inode、版本、放置、租约和生命周期元数据；文件数据不经过 Meta。
- `afs-node` 提供 FUSE 挂载、本地存储、P2P 读取、缓存和恢复所需的服务。
- `DistributedFs` 是通用 DFS 路径，包含 chunk、版本、replica、读计划和修复机制。
- `OwnerFs` 是 1 到 4 节点的小集群 workspace 路径。workspace 有 Home 节点，本地访问走普通文件，远端访问回到 Home。
- OwnerFs workspace bind mount 是当前实际试用场景：显式开启后，把 Home 上 workspace 的底层真实目录 bind 到 OwnerFs FUSE 根下对应一级目录。普通发行配置仍默认关闭。

## 当前边界

G1 历史试用范围保持 8/8 关闭；当前快照不重开、不重标历史结论。a103 试用包证明的是有限范围：真实 Home bind ON、UID501/UID502 权限检查、Owner64KiB 与 DFS64MiB 在 local-file Meta 下正常全停重启恢复，以及正常退出。它不是完整 POSIX、完整远端标准、性能达标或复杂可靠性通过。

仍未完成的主要项包括：普通 OwnerFs 本地/远端性能双目标、DFS 对 3FS 的三同步持久副本对照、完整远端 POSIX、复杂可靠性、多 Meta、etcd/Redis 后端验收，以及官方 `fuser` 无私有补丁迁移。

## 文档入口

1. [文档首页](docs/README.md)
2. [当前计划](docs/development/plan.md)
3. [架构总览](docs/architecture.md)
4. [OwnerFs 机制](docs/architecture/ownerfs.md)
5. [测试与验收](docs/testing/acceptance.md)
6. [部署与试用](docs/deployment/trial.md)

## 构建

Rust 工具链由 `rust-toolchain.toml` 固定。普通构建只生成产品二进制，不包含测试探针。

```sh
cargo build --release --locked --bin afs-node --bin afs-meta
```

验收流程需要 workspace 探针时单独构建：

```sh
cargo build --release --locked --example afs-workspace-probe
```

文件系统相关构建、测试和运行结论只在 Linux 上成立；macOS 只用于编辑和 VM 编排。
