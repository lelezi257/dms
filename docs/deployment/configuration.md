# 配置说明

配置分为 Meta、Node、TLS、挂载目录和 OwnerFs workspace bind。示例文件在 `examples/` 和发布包的 `templates/` 目录中。

## Meta 后端

当前主线按简单到复杂推进：

1. `memory`：一次性演示，重启不保留状态。
2. `local-file`：当前可靠性基线，中心节点可正常重启恢复。
3. `etcd`：保留实现，资源和可靠性专题后置。
4. `redis`：最低优先级，后置。

试用建议使用 `local-file`：

```toml
id = "meta-ctl"
fs = "all"
meta_store = "local-file"
data_dir = "/var/lib/afs/meta"
grpc_listen = "127.0.0.1:7400"
rest_listen = "127.0.0.1:7401"
```

`memory` 只能用于临时演示。若使用 `memory`，必须在报告中明确“Meta 重启后不保留状态”。

## Node 基本配置

以下仅为字段节选，不能直接作为完整可运行配置。请使用 `afs-trial-config single --backend local-file` 生成包含 TLS、advertise endpoint 和 trusted node certificates 的完整配置：

```toml
id = "node-a"
grpc_listen = "127.0.0.1:7500"
rest_listen = "127.0.0.1:7501"
meta_endpoint = "http://127.0.0.1:7400"
fs = "all"
data_mode = "grpc"
data_dir = "/var/lib/afs/node-a"
uds_path = "/run/afs/node-a.sock"
ownerfs_mount = "/mnt/afs/ownerfs"
dfs_mount = "/mnt/afs/dfs"
timeout_ms = 5000
log_level = "info"
trace_enabled = false
```

多节点时，每个 Node 必须使用不同的 `id`、监听端口、`data_dir`、`uds_path` 和挂载目录，并在 Meta 的 TLS 信任配置中绑定对应证书。

## TLS

试用包的 `install.sh` 会生成本地 TLS 引导证书。多节点时建议使用：

```sh
/opt/afs/bin/afs-trial-config cluster --backend local-file \
  --meta-host 192.168.109.11 \
  --node node-a=192.168.109.12 \
  --node node-b=192.168.109.13 \
  --output /tmp/afs-trial-r2
```

然后把 `<output>/meta/etc` 和 `<output>/<node-id>/etc` 分发到对应主机。不要复用旧私钥、旧 PID 文件或旧 runtime 目录来伪造新环境。

## OwnerFs workspace bind ON 配置

OwnerFs workspace bind mount 的正式名称与容器无关。它把 Home 上 workspace 的底层真实目录 bind 到 OwnerFs FUSE 根目录下对应一级目录。

普通发行默认关闭：

```toml
experimental_ownerfs_workspace_bind = false
```

当前试用场景显式开启：

```toml
experimental_ownerfs_workspace_bind = true

[ownerfs_workspace_bind]
workspace = "agent1"
```

开启条件：

- `agent1` 必须是一个已存在的本地 Home workspace；
- 目标必须是 OwnerFs FUSE 根下的一级目录，例如 `/mnt/afs/ownerfs/agent1`；
- 源必须是 Home 上的底层真实目录，不能把 FUSE 目录自身 bind 到别处后宣称绕过 FUSE；
- 同一 Node 只开启一个管理员控制的 host bind entry；
- 不要同时开启 legacy `experimental_native_workspace` 和 `experimental_ownerfs_workspace_bind`。

仍需保留的语义：授权、Home/root/epoch 身份核验、权限、错误传播、数据新鲜度、close-to-open、正常卸载和受管引用排空。当前实现不能立即撤销已有 native FD、mmap、描述符转移或二级 clone；停机或 root/epoch 变化前先停止受管用户。

## Legacy runc 适配开关

`experimental_native_workspace` 是历史命名，控制 runc 私有 namespace 实验适配，不是当前核心能力名称：

```toml
experimental_native_workspace = false
```

保留该字段是为了兼容旧配置，不应把它当成新的 OwnerFs workspace bind 核心开关。

## 日志与追踪

默认使用 `log_level = "info"`、`trace_enabled = false`。性能测试不要长期开启全量 TRACE；若开启 tracing，必须限制采样比例和日志空间，并在结果中记录配置。
