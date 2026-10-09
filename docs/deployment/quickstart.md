# 快速开始

本页给出最小可运行路径。文件系统和 Rust 构建结论只在 Linux 上成立；macOS 只用于编辑和 VM 编排。

## 方式一：从源码构建

在 Linux ARM64 环境中执行：

```sh
cargo build --release --locked --bin afs-meta --bin afs-node
```

产物位于 `target/release/afs-meta` 和 `target/release/afs-node`。普通构建只包含产品二进制，不包含测试容器探针。验收流程需要 workspace 探针时，按需单独构建：

```sh
cargo build --release --locked --example afs-workspace-probe
```

该探针只给验收流程使用，不放入普通试用包。

## 方式二：安装阶段性试用包

当前阶段性试用包是 [afs-bind-a103a2f](https://github.com/lelezi257/dms/releases/tag/afs-bind-a103a2f)。它适合验证有限的 OwnerFs workspace bind ON 场景和 local-file Meta 正常重启恢复，不代表完整 G2/G3 验收。

在目标 Linux 主机上下载并解压发布包后：

```sh
sha256sum -c SHA256SUMS
sudo ./install.sh --prefix /opt/afs --config-dir /etc/afs --state-dir /var/lib/afs --run-dir /run/afs --log-dir /var/log/afs --mount-root /mnt/afs
```

安装脚本会：

- 安装 `afs-meta`、`afs-node`、`afs-processctl`、`afs-trial-config`、`afs-selfcheck` 和 `dep02-smoke.sh`；
- 初始化本地 TLS 引导证书；
- 生成默认 `meta.toml`、`node.toml`、兼容 `node-dfs.toml` 和 `node-ownerfs.toml`；
- 保留已存在的配置和数据目录。

安装后可以先运行只读自检：

```sh
sudo /opt/afs/bin/afs-selfcheck --prefix /opt/afs --config-dir /etc/afs
```

## 单机试运行

生成或刷新单机配置：

```sh
sudo /opt/afs/bin/afs-trial-config single --backend local-file --config-dir /etc/afs --state-dir /var/lib/afs --run-dir /run/afs --mount-root /mnt/afs --force
```

启动服务：

```sh
sudo /opt/afs/bin/afs-processctl --prefix /opt/afs --config-dir /etc/afs --run-dir /run/afs --log-dir /var/log/afs start all
```

查看状态：

```sh
sudo /opt/afs/bin/afs-processctl --prefix /opt/afs --config-dir /etc/afs --run-dir /run/afs --log-dir /var/log/afs status all
findmnt /mnt/afs/ownerfs
findmnt /mnt/afs/dfs
```

停止服务：

```sh
sudo /opt/afs/bin/afs-processctl --prefix /opt/afs --config-dir /etc/afs --run-dir /run/afs --log-dir /var/log/afs stop all
```

## 开启 OwnerFs workspace bind

普通配置默认 OFF。当前实际试用场景需要显式开启，步骤见 [配置说明](configuration.md#ownerfs-workspace-bind-on-配置)。开启前必须已经有一个本地 Home workspace，并确认要覆盖的是 OwnerFs FUSE 根下的一级目录，例如 `/mnt/afs/ownerfs/agent1`。

## 下一步

- 配置字段和 ON 场景边界见 [配置说明](configuration.md)。
- 启停、恢复、日志和清理见 [运维说明](operations.md)。
- 当前包身份、已验证范围和未完成项见 [试用说明](trial.md)。
