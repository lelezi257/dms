# 发布包：安装与独立编程

本页针对 GitHub Release 发布的 **0.1.0**。Rust SDK 归档为 `dms-client-0.1.0.crate`，服务为 `dms-server-0.1.0-linux-${arch}.tar.gz`，`${arch}` 由构包机器决定。Go SDK 通过模块 tag `sdk/go/v0.1.0` 获取。不需要单独部署或安装 DMS common 包。下载入口和校验值见 [0.1.0 发布说明](release-0.1.0.md)。

当前验收目标是发布页列出的 Linux 架构与 Ubuntu 24.04；其它平台未据此获得支持承诺。仅本地内存可靠性，Node 重启可能丢失 value；元数据 WAL 不会恢复 value。正常删除与旧版本回收已有实现，但永久失联 Node 或未归还的共享写权仍可能阻塞释放。多 Meta HA、RDMA/UB、L2 不在本版本承诺内。

## 1. 先进入 Linux，再执行安装

macOS 只负责启动并进入已准备的虚拟机。已有 dms-dev 环境可使用：

```bash
limactl start dms-dev
limactl shell --workdir /tmp dms-dev
```

下面全部在 Linux 终端执行。把发布 tar 与同名 `.sha256` 放在当前目录，先校验再解压。这里的目录属于本次安装，不使用开发源码下的 log/data。

```bash
sha256sum -c dms-server-0.1.0-linux-aarch64.tar.gz.sha256
tar -xzf dms-server-0.1.0-linux-aarch64.tar.gz
cd dms-server-0.1.0-linux-aarch64
sha256sum -c SHA256SUMS
python3 -m json.tool SERVER-PACKAGE-MANIFEST.json | head
cp config/dms.env.example config/dms.env
```

需要 Bash、curl 与基础 Linux 工具；只运行服务不需要 Rust 或 protoc。重复安装先选一个新的目录，不覆盖正在运行的包。

## 2. 配置并手动启动

编辑 `config/dms.env`。单 VM 保持 `DMS_NODE_IP=127.0.0.1`；默认业务端口 Node 19200、Meta 19300，状态端口 19000/19100。如果这些端口已被使用，要把 bind、endpoint 和后续验证命令一起修改。

```bash
./scripts/cluster.sh meta start
./scripts/cluster.sh node start
./scripts/cluster.sh meta status
./scripts/cluster.sh node status
```

脚本检查 ready；再次 start 不会偷偷停止旧进程。数据在 `data/meta-journal`，日志在 `log`，socket/PID 在 `run`。停止只针对本包记录且核对过可执行文件的进程，不删除数据。

```bash
export DMS_ENDPOINT=http://127.0.0.1:19200
./bin/sdk-kv set example/key hello
./bin/sdk-kv get example/key hello
./bin/sdk-kv del example/key
```

这是随组件提供的诊断程序。应用开发使用下一节的 SDK，而不是调用这个命令代替 API。

## 3. 用户程序只依赖 SDK

Rust 用户需要 Rust 1.95.0 和 C 链接工具链；**不需要 protoc**。发布 `.crate` 已包含生成的协议代码，SDK 公共类型仍是原生 Rust 类型。

0.1.0 未上传 crates.io，因此 `cargo add dms-client` 不会从 crates.io 得到本项目。在线消费者使用 `v0.1.0` Git tag；发布包验收仍使用仅回环监听的临时 Cargo 仓库，证明 `.crate` 本身可以被隔离消费。

先打开另一个 Linux 终端，进入解压的服务包目录，把 SDK `.crate` 放在该目录，再启动随包提供的临时仓库工具（Python 3.11+）：

```bash
python3 scripts/candidate_registry.py dms-client-0.1.0.crate /tmp/dms-candidate-index --port 26880
```

`/tmp/dms-candidate-index` 必须是不存在的新目录；已有则换一个名字，不要删除别人的目录。保持这个终端运行，工具显示 SDK 名称、版本、SHA；SDK 编译完成后 Ctrl-C 可关闭仓库，不会影响 Node。源码目录对应工具是 `scripts/release/candidate_registry.py`。

回到应用终端：

```bash
curl -fsS http://127.0.0.1:26880/config.json
cargo new my-dms-app
cd my-dms-app
mkdir .cargo
```

将 `Cargo.toml` 改为：

```toml
[package]
name = "my-dms-app"
version = "0.1.0"
edition = "2024"

[dependencies]
dms-client = { version = "=0.1.0", registry = "dms-candidate" }
```

在应用的 `.cargo/config.toml` 指定当前测试仓库：

```toml
[registries.dms-candidate]
index = "sparse+http://127.0.0.1:26880/"
```

应用 `src/main.rs`：

```rust
use dms_client::{ClientOptions, DmsClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = DmsClient::connect("http://127.0.0.1:19200", ClientOptions::default())?;
    client.set("app/key", b"hello")?;
    assert_eq!(client.get("app/key")?.as_deref(), Some(&b"hello"[..]));
    client.del("app/key")?;
    assert!(client.get("app/key")?.is_none());
    Ok(())
}
```

```bash
cargo run
```

没有 `../../sdk` 路径依赖，没有第二个 DMS 包，也不用导入 protobuf。正式发布到选定仓库后只调整 Cargo 仓库配置，不改这些业务调用。

共享内存使用同一套公开操作：endpoint 改为 `unix://` 加安装目录中 `run/dms-worker.sock` 的绝对路径，并设置 `ClientOptions { shared_memory: Some(true), ..Default::default() }`。只允许可信本地进程；TCP 不传 FD。

## 4. 日志、指标、Trace 与界面

服务日志直接读 `log/dms-node.log`、`log/dms-meta.log`。Node/Meta 的状态端口提供 `/metrics`；以下观测栈需要 Docker 与 Compose，镜像首次运行需要下载。

若要做采样为 100% 的手动 Trace 实验，先在配置中设 `DMS_TRACING_ENABLED=true`、`DMS_TRACING_SAMPLE_RATIO=1`、`DMS_LOG_LEVEL=debug`，再启动服务。正常默认 Trace 关闭，不要把实验配置当生产建议。

完整指标验收还包含 SHM 样本：在 `config/dms.env` 把 `DMS_CLIENT_ENDPOINT` 设为 `unix://` 加当前安装目录的绝对路径，例如 `unix:///opt/dms/run/dms-worker.sock`。不要照抄不属于自己安装目录的路径。

```bash
./scripts/metrics-targets.sh single
./scripts/observability.sh up
./scripts/cluster.sh client start
```

`client start` 运行随包提供的示例宿主，演示 SDK Metrics/Trace 接入，不是 SDK 必须独立部署一个进程。Grafana 默认端口 3000；开发默认账号 `admin` / `dms-dev`。这个栈仅用于可信开发环境，不能直接暴露到公网；正式部署需另行配置认证、TLS、防火墙和存储策略。

```bash
./scripts/verify_metrics.sh
./scripts/verify_logs.sh
./scripts/verify_tracing.sh
```

打开 Grafana 的 Explore：`DMS Prometheus` 查询指标，`DMS Loki` 查询 `{service_name="dms-node"}` 日志，`DMS Tempo` 按 Service Name=`dms-client` 与 Span Name=`dms.client.set` / `dms.client.get` 搜索链路，或使用验收输出的 Trace ID。源码仓 `docs/observability.md` 提供更完整说明，但运行包不依赖源码教程。修改观测端口时同步设置 `DMS_*_PORT` 和验证脚本 `DMS_*_ADDRESS`，不能只改服务端口。

同一 VM 已有观测栈时，额外设置 `DMS_OBSERVABILITY_PROJECT` 为新的 Compose 项目名，并调整端口；启动和停止必须使用相同项目名，避免操作其它实验的容器。

## 5. 停止但保留数据

```bash
./scripts/cluster.sh all stop
./scripts/observability.sh down
```

不会删除 Meta journal、日志或观测卷。Node 的 value 是内存数据；停止后不要误以为 journal 保证了 value 恢复。

## 6. 升级、回滚与卸载

发布包是目录级安装。每次升级都解压到一个新目录，不覆盖旧目录。

升级步骤：

1. 在旧目录执行 `./scripts/cluster.sh all stop`，确认旧进程停止。
2. 解压新 tar，执行 `sha256sum -c SHA256SUMS`。
3. 只复制确认需要保留的 `config/dms.env` 与 Meta journal；不要复制旧 `run/` 或 PID 文件。
4. 在新目录启动 Meta/Node，并重新执行 SET/GET 或业务验收。

回滚步骤：停止新目录，回到旧目录启动。若新版本已经写入旧版本不能识别的 journal，不要假定可无损回滚，先保留现场并定位。

卸载步骤：先 stop 服务和观测栈；确认不再需要本次实验数据后，再删除安装目录、`data/`、`log/`、`run/` 与观测 Docker volume。普通 stop 不删除数据，避免误删可恢复证据。

## 7. 维护者：构包与门禁

以下是维护者命令，必须进入完整源码的 Linux 环境，不能在二进制运行包目录执行：

```bash
bash scripts/release/check.sh
python3 scripts/release/dependency_inventory.py --output artifacts/third-party
./scripts/build.sh
python3 scripts/package_sdk.py
./scripts/package.sh
```

包输出到独立、版本化的 `artifacts/` 目录；SDK 内部实现由原源码机械归并，不维护第二套手写实现。CI 平台可直接调用 `scripts/release/check.sh`，它不依赖已部署服务、不发布远端；发布包安装门禁另行执行，不能以单元测试代替。本地 Go SDK RC 可用 `python3 scripts/sdk/package_go.py --output 输出目录` 生成 module proxy。

维护者执行 `bash scripts/release/accept.sh --sdk-crate SDK.crate --server-archive SERVER.tar.gz --output-dir 新证据目录` 可启动隔离 Ubuntu 消费者。它只挂载发布输入、结果、第三方依赖快照和 Rust 工具链，不挂载项目源码，也不安装 protoc；消费者从临时 Cargo 仓库下载 SDK。`--keep-running` 为之后单独验证观测栈保留服务，基础验收本身不代表 Grafana/Loki/Tempo 验收通过。参数与清理行为见 `--help`。

RC 验收可继续使用 `candidate_manifest.py` 汇总本地制品；正式上传完成后使用 `release_manifest.py` 记录远端资产名、tag、commit 和 SHA256。两者不能混用。

```bash
python3 scripts/release/candidate_manifest.py \
  --rust-sdk-crate "$SDK_CRATE" \
  --go-proxy "$GO_PROXY" \
  --server-archive "$SERVER_TAR" \
  --third-party-inventory "$THIRD_PARTY_DIR/inventory.json" \
  --acceptance-result "$ACCEPT_DIR/results/consumer-result.json" \
  --output "$RC_ROOT/release-candidate-manifest.json"
```

第三方清单用 `python3 scripts/release/dependency_inventory.py --output 专用目录` 生成；缺失或校验失败会非零退出并保存缺项。通过 `DMS_THIRD_PARTY_DIR` 将该目录传给构包脚本；正式构包时缺少它会直接失败。清单是工程合规材料，不替代法律审批。
