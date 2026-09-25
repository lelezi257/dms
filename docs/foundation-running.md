# 运行 AFS 正式基础框架

本轮提供可以继续写真实业务的进程、接口、配置和传输底座。Ping、8 字节普通文件读写和 FUSE 分派是验证负载。**尚不能用它存放 workspace 或发布镜像。** FUSE 的创建入口故意返回 `ENOSYS`，不会假装文件已创建。

## Linux 构建

Rust 版本由 rust-toolchain.toml 锁定。普通构建需要 C 工具链和 protobuf-compiler；启用 RDMA 再安装 libibverbs-dev。运行 FUSE 需要 `/dev/fuse` 和 fusermount3；容器需放行该设备和挂载权限。产品 binary 不需要 Python；Python 仅用于仓库 E2E 测试。

```sh
cargo build --locked --workspace --bins --examples
# 只编译一个后端，关闭另一个后端的模块
cargo build --locked -p afs --no-default-features --features ownerfs
cargo build --locked -p afs --no-default-features --features blobfs
# 两个后端及真实 RDMA 机制
cargo build --locked --workspace --all-features --bins --examples
```

默认编译 OwnerFs 和 BlobFs。`--fs ownerfs|blobfs|all` 决定运行时 namespace；未指定时使用当前编译进来的后端。指定未编译后端立即报错。无后端构建可保留 Meta；Node 不能启动。RDMA 是独立编译 feature，不随 SDK 引入。

配置顺序与 main 一致：默认值 < TOML < 显式 CLI。未知 TOML 字段报错。`--print-config` 解析并检查配置，不启动进程。配置样例在 examples/meta.toml 和 examples/node.toml。

## 启动及观察

使用专用临时目录；Node 不会覆盖已有 UDS socket。先建挂载目录：

```sh
mkdir -p /tmp/afs-node-b/mnt
./target/debug/afs-meta --config examples/meta.toml
```

另外两个终端分别启动 Node：

```sh
./target/debug/afs-node --id node-a \
  --grpc-listen 127.0.0.1:7500 --rest-listen 127.0.0.1:7501 \
  --uds-path /tmp/afs-node-a/local.sock --data-dir /tmp/afs-node-a/data

./target/debug/afs-node --id node-b \
  --grpc-listen 127.0.0.1:7600 --rest-listen 127.0.0.1:7601 \
  --uds-path /tmp/afs-node-b/local.sock --data-dir /tmp/afs-node-b/data \
  --meta-endpoint http://127.0.0.1:7400 --peer-endpoint http://127.0.0.1:7500 \
  --fs all --data-mode grpc --mount /tmp/afs-node-b/mnt
```

```sh
curl http://127.0.0.1:7401/v1/ping
curl http://127.0.0.1:7601/health
curl -X POST http://127.0.0.1:7601/v1/diagnostics
curl http://127.0.0.1:7601/metrics
./target/debug/examples/local_roundtrip /tmp/afs-node-a/local.sock sdk-eight
ls /tmp/afs-node-b/mnt
# 以下命令预期报 Function not implemented，同时日志显示 FUSE → VFS → OwnerFs。
touch /tmp/afs-node-b/mnt/ownerfs/hello
```

`/v1/diagnostics` 显式执行 Node→Meta Ping、Node→Node 控制 Ping、同一数据 adapter 的 8 字节写入/读回。诊断文件在对端 data_dir/diagnostics 下，与未来业务 namespace 分开。它不会在每次 FUSE 操作时运行，也不证明根授权或发布已实现。数据 I/O ACK 表示普通文件 I/O 已完成，本轮没有宣称逐写掉电持久。

日志为 JSON，模块事件可直接对应 `src/node/fuse.rs`、`vfs.rs`、`rpc/peer.rs`、`rpc/data.rs`、`api/local.rs` 和 `meta/rpc.rs`。指标通过各进程 REST `/metrics` 导出。`--trace-enabled true --trace-sample-ratio 1 --trace-endpoint http://127.0.0.1:4317` 开启 OTLP；SDK 服从宿主 Trace 配置，不创建全局 subscriber。

SIGINT/SIGTERM 触发停止接入、清理服务、卸载本进程 FUSE 与本进程 UDS。异常 SIGKILL 后旧 socket 可能残留；确认旧进程已死并检查路径后清理，不自动删除无法证明归属的 socket。已有 FD 的透明恢复不在这个基础框架中。

## 通道边界

- SDK：UDS 控制 + 每次操作独立的 sealed-size memfd，通过 SCM_RIGHTS 传递文件描述符；服务端校验授权、长度与 seals 后读写共享缓冲。当前使用有界的字节拷贝，不宣称零拷贝优化完成。无 gRPC 内容备用通道，普通 POSIX 程序不需要 SDK。
- 节点间：`--data-mode grpc|rdma|auto`。RDMA 时两个 Node 配置各自 `--rdma-device DEVICE`；仍使用 gRPC 命令，写时服务端 READ 客户端内存、读时服务端 WRITE 客户端内存。
- 自动选择只发生在建连；写操作结果不明时不换通道重放。取消等待不会缩短在途缓冲的生命期。
- RXE 可验证单边语义、错误及回收；不能替代真实网卡性能/跨机器验证。
- 默认端口仅监听回环。基础示例用于可信测试环境；身份认证、租户隔离、根授权和生产访问控制不能由 Ping/诊断接口替代。

## 自动化验收

```sh
cargo fmt --all --check
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features
bash tests/feature-matrix.sh
# 有 FUSE 设备与挂载权限时，显式运行真实挂载测试
cargo test --all-features --test fuse_contract -- --ignored
# 先构建两个进程和验收示例，包括 OTLP collector 与 SDK 调用程序
cargo build --workspace --all-features --bins --examples
python3 tests/foundation_e2e.py --bin-dir target/debug --output /tmp/afs-evidence
# 有已配置 RDMA/RXE 设备时
python3 tests/foundation_e2e.py --bin-dir target/debug --rdma-device DEVICE --output /tmp/afs-rdma-evidence
```

RDMA 握手版本 1 使用 `NegotiateData` + RDMA `SEND_WITH_IMM` 就绪探测，已移除 `ReadyData`；升级时两端一起更新。单边内容传输和 gRPC 命令/结果的分工不变。

真实 RXE 生命周期测试另用 `AFS_TEST_RDMA_DEVICE=DEVICE cargo test --all-features --test rdma_lifecycle -- --ignored`，覆盖探测后的实际搬运、未就绪拒绝、取消后拒绝复用和会话过期。没有设备时不执行这组显式测试。

每次运行输出 JSON 验收结果及独立进程日志。实际验收覆盖和限制见 status.md；架构目标仍由 requirements.html/architecture.html 定义，不能将目标能力解释为本轮已经提供。
