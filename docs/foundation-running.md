# 构建、运行与验收

权威构建和运行环境是 Linux。Rust 版本由 `rust-toolchain.toml` 固定。构建需要 C 工具链和 `protobuf-compiler`；运行 FUSE 需要 `/dev/fuse` 与 `fusermount3`，容器还需放行设备和挂载权限。

## 构建

```sh
cargo build --locked --workspace --bins --examples
cargo build --locked -p afs --no-default-features --features ownerfs
cargo build --locked -p afs --no-default-features --features dfs
cargo build --locked --workspace --all-features --bins --examples
```

默认编译 OwnerFs 和 DFS。`--fs ownerfs|dfs|all` 决定运行哪些后端；两个后端分别使用 `--ownerfs-mount` 和 `--dfs-mount`，路径必须不同。每个 mount 创建独立 FUSE session。指定未编译的后端会立即报错。

配置顺序为默认值 < TOML < 显式 CLI。未知 TOML 字段报错。配置样例位于 `examples/meta.toml` 和 `examples/node.toml`。

## DFS R=1 真实 FUSE 验收

先构建只含 DFS 的进程：

```sh
cargo build --locked -p afs --no-default-features --features dfs --bins
sudo python3 scripts/dfs/r1_e2e.py \
  --bin-dir target/debug \
  --work-dir /tmp/afs-dfs-r1
```

脚本启动 local-file MetaStore 和 DFS-only Node，挂载独立 DFS mount，然后执行：

```text
create /hello.txt
→ 两次 write
→ fsync
→ close
→ reopen
→ read + stat
→ 检查磁盘上唯一 immutable Chunk 的内容
```

通过结果以 JSON 输出。失败时保留工作目录和进程日志供诊断。

## 独立进程示例

```sh
mkdir -p /tmp/afs-dfs/mnt

./target/debug/afs-meta \
  --fs dfs \
  --meta-store local-file \
  --data-dir /tmp/afs-meta \
  --grpc-listen 127.0.0.1:7400 \
  --rest-listen 127.0.0.1:7401

./target/debug/afs-node \
  --id node-a \
  --fs dfs \
  --meta-endpoint http://127.0.0.1:7400 \
  --advertise-endpoint http://127.0.0.1:7500 \
  --grpc-listen 127.0.0.1:7500 \
  --rest-listen 127.0.0.1:7501 \
  --uds-path /tmp/afs-dfs/local.sock \
  --data-dir /tmp/afs-dfs/data \
  --dfs-mount /tmp/afs-dfs/mnt
```

`SIGINT`/`SIGTERM` 触发停止接入、清理服务、卸载本进程 FUSE 和 UDS。异常退出可能留下 mount 或 socket；确认旧进程结束后再清理。

## 验证命令

```sh
cargo fmt --all --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
bash tests/feature-matrix.sh
```

OwnerFs 三节点验收和已有性能证据见 `docs/reviews/`。DFS R=1 的现行能力与限制见[当前状态](current-status.md)。
