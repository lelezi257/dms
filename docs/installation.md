# 安装与构建

当前是 0.1 开发预览的源码使用方式，还没有经过正式发布验收的下载地址或 Cargo Registry 版本。不要把示例中的 workspace 依赖当成已发布 SDK。

## 1. 准备 Linux 环境

有仓库权限的开发者可先取得源码（Git 凭据使用本地凭据管理器，不写进 URL）：

```bash
git clone https://github.com/lelezi257/dms.git source
cd source
```

保留目录名 `source` 可直接使用下面的 Lima 开发脚本。普通 Linux 环境不限制检出目录名。

构建、服务、Docker 和测试都在 Linux 内运行。macOS 仅用于编辑、阅读和启动 VM。随源码提供的 Lima 配置是 Apple Silicon / aarch64、Ubuntu 24.04、4 核、8 GiB 内存、40 GiB 磁盘；不是通用 x86 VM 配置。

Mac 先安装好 Lima。在源码根目录（有 `Cargo.toml` 和 `scripts/`）执行：

```bash
./scripts/vm.sh up
./scripts/vm.sh shell
```

`up` 首次创建 `dms-dev`，挂载源码上一级目录到 `/workspace/dms`；`shell` 直接进入 `/workspace/dms/source`。**当前 Lima 脚本要求源码目录名为 `source`**：独立 checkout 时也放在一个父目录的 `source/` 下。它不需要父目录里存在研究资料，但还不能自动适配其它源码目录名。已有同名 VM 挂载不同项目时不要直接复用，可先设置 `DMS_DEV_VM` 为一个新名字。不要假定存在 `n1/n2/n3`，这个脚本只管理一台开发 VM。

已有 Linux 主机也可把完整源码放在自己的目录中，无需 Lima。以下步骤从源码根目录执行。

## 2. 配置和构建

在 Linux 终端执行：

```bash
source scripts/env.sh
export CARGO_INCREMENTAL=0
./scripts/build.sh
```

首次加载环境会用 `sudo apt-get` 安装 C 编译器、`pkg-config`、`protoc` 等，并安装 Rust 1.95.0、rustfmt、Clippy；需要网络和 sudo。脚本会切换到源码根目录。首次编译和依赖缓存需要额外磁盘空间，先用 `df -h` 查看空间。

产物位于 VM 私有的 `$CARGO_TARGET_DIR/release/`：

| 产物 | 用途 |
| --- | --- |
| `dms-node` | 存储数据、提供 Worker/Peer 服务 |
| `dms-meta` | 位置、版本、提交和节点状态 |
| `dms-health` | 命令行探活工具 |
| `examples/sdk_kv` | 使用 SDK 的手工 SET/GET/DEL 示例，不是 SDK 服务进程 |
| `examples/metrics_host` | 自带 Registry/HTTP 的 SDK 宿主示例 |

接着按[单 VM 手工教程](local-single-vm-manual.md)启动真实进程。

## 3. 验证与打包边界

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

已有 `./scripts/package.sh` 可生成本机架构的实验运行包，包含服务二进制、示例 CLI、运行脚本和观测配置。它不是 Rust SDK 的 Cargo 发布包；正式构包、干净环境安装与远端发布是独立验收项，不能由 `cargo package --list` 替代。

## 4. 安全边界

教程使用明文 gRPC 和本机开发观测栈。不要把它直接暴露到公网；当前不承诺多租户安全隔离、生产 TLS 配置闭环或 Meta HA。Meta WAL 持久化的是元数据，不会把 Node 内存中的用户 value 自动持久化。
