# DMS 0.1.0 候选发布总入口

本页把 0.1.0 候选版本的用户入口、制品、限制和验收流程放在一起。它不是 GitHub Release 公告，也不表示已经上传 crates.io 或 Go 公开模块版本；远端发布必须另行授权。

## 0. G003 本地候选验收状态

当前 G003 候选包已经完成本地制品生成、隔离消费、单 VM/三 VM 运行、可靠性回归和冻结性能门禁；正式 tag / GitHub Release 仍需人工门禁。最终审阅入口在源码外层 `outputs/stages/g003-rc-delivery.html`，机器证据在 `evidence/g003-rc-delivery/final/`、`evidence/g003-rc-delivery/final-current/` 和 `evidence/g003-rc-delivery/performance/`。

本轮最终拷贝到交付目录的候选制品为：

| 制品 | SHA256 | 说明 |
| --- | --- | --- |
| `dms-client-0.1.0.crate` | `c2e0023439cf5e0e0cc5d8abe869a44d6bf3eefdfb5c8aeb2b2a63c79e50e8a5` | Rust SDK 候选 crate；独立消费者不需要源码或 protoc。 |
| `dms-server-0.1.0-linux-aarch64.tar.gz` | `63859e2c35b3b35dd7c2a05a122c4ec3a7e728ac38925c457e5db1df6c7234c9` | Node/Meta/health/sdk-kv 运行包。 |
| Go SDK module proxy zip | `1ae8082f60ccd018556a1311070a932844350758ba7c8340b29de821edc15766` | 本地 module proxy 版本 `v0.1.0-rc.bb627ce0afb4f940`，仍是 preview。 |
| `juicefs-dms-0.1.0-rc-linux-arm64.tar.gz` | `9d478f54ef54f9f6b729f63d0e273e1dd497defb96a06f9f296ef3357e291cde` | JuiceFS DMS preview 运行包。 |

候选总清单已经按最终交付目录重新生成，路径和 hash 与上表一致。冻结性能门禁在相同规格的三台 VM 上以每项至少 100 个样本重跑，14/14 项通过；早期 11/14 结果来自低规格 VM 和代理环境污染，保留为诊断证据，不是最终结论。第三方依赖清单仍有 9 项待发布/法务审阅；这不阻止本地 RC 技术验收，但阻止把它直接描述为已经完成正式发布合规审查。

## 1. 本候选版本包含什么

| 制品 | 面向谁 | 当前状态 | 机器清单 |
| --- | --- | --- | --- |
| Rust SDK `.crate` | Rust 应用开发者 | 候选；消费者只依赖 `dms-client`，不依赖 protobuf/common crate | `scripts/package_sdk.py` 生成 `sdk-package-manifest.json` |
| Server tar | 运维/测试部署 | 候选；包含 `dms-node`、`dms-meta`、`dms-health`、示例工具、运行脚本、观测配置 | 包内 `SERVER-PACKAGE-MANIFEST.json`、`MANIFEST.txt`、`SHA256SUMS` |
| Go SDK module proxy | Go 接入验证 | 预览；覆盖基础对象、随机写、批量、Hash/KKV、Reader/用户 buffer；不作为 0.1.0 正式产品承诺 | `scripts/sdk/package_go.py` 输出 JSON |
| 第三方依赖清单 | 发布/合规检查 | 候选门禁输入 | `scripts/release/dependency_inventory.py` 生成 `inventory.json` |
| 候选发布总清单 | 维护者/验收系统 | 汇总上述制品，不上传远端 | `scripts/release/candidate_manifest.py` 生成 |

用户产品面是 SDK + 服务组件。`common/`、`protocol/` 是源码内部工程边界，不是用户要单独安装的第三个包。

## 2. 能力、限制与兼容矩阵

| 项目 | 0.1.0 RC 结论 |
| --- | --- |
| 支持平台 | Linux aarch64/x86_64 按本机架构构包；当前验收优先 Ubuntu 24.04。 |
| Rust SDK | 支持 `set/get/del/stat/scan`、批量、随机写、KKV、条件写、范围读和本地 SHM 显式接口。 |
| Go SDK | 纯 Go 预览，支持 `Set/Get/Del/Stat/Scan/GetInto/GetReader/SetFrom/SetRange/MSet/MGet/HSet/HGet/HMGet/HGetAll/HDel/HScan/HWriteAt`；不等于 Rust 全量能力。 |
| Python/C++ SDK | 规划中，不在 0.1.0 RC 交付范围。 |
| 部署形态 | 单 Meta + 多 Node；单 VM 和 3 VM 手动部署可验证。 |
| 数据可靠性 | 仅本地内存策略。Meta WAL/snapshot 恢复元数据，不恢复 Node 内存 value。 |
| 传输 | 明文 gRPC/TCP、同机 UDS + SHM。TLS、RDMA/UB、L2/对象存储分层未实现。 |
| 观测 | Node/Meta 本地日志、Prometheus Metrics、可选 Trace；SDK 不抢宿主全局观测后端。 |
| 安全边界 | 开发可信环境；SHM 只给可信本地进程；没有生产 TLS/多租户隔离闭环。 |

更完整的边界见 [能力与限制](product.md)。安装前先接受这些限制，尤其不要把候选包当成生产持久存储。

## 3. 用户阅读路径

| 任务 | 入口 |
| --- | --- |
| 从候选包安装并写 Rust 程序 | [候选包安装与独立编程](release-installation.md) |
| 单 VM 手动启动和 SET/GET | [单 VM 教程](local-single-vm-manual.md) |
| 三 VM 部署和跨 Node 读 | [三 VM Metrics 与部署教程](metrics-three-node-manual.md) |
| Rust API 语义 | [Rust SDK 编程](rust-sdk.md) |
| Go API 语义 | [Go SDK 编程](go-sdk.md) |
| 配置优先级和参数 | [配置](configuration.md) |
| 日志、指标、Trace 和界面 | [观测](observability.md) |
| 故障定位 | [排障](troubleshooting.md) |

## 4. 安装、升级、卸载

候选包是一个目录级安装，不写系统服务管理器。

- 安装：校验 tar 的 companion `.sha256`，解压到新的安装目录，执行包内 `sha256sum -c SHA256SUMS`，复制并编辑 `config/dms.env`。
- 升级：先在旧目录执行 `./scripts/cluster.sh all stop`；解压新目录；只迁移明确需要保留的 `config/dms.env` 和 Meta journal；启动新目录并重新跑业务验收。不要覆盖正在运行的旧目录。
- 回滚：停止新目录，回到旧目录重新 start。若新版本已经写入旧版本不能识别的 journal，不能假定可无损回滚。
- 卸载：停止服务和观测栈后，按需要删除安装目录、`data/`、`log/`、`run/` 和 Docker volume。普通 stop 不删除数据。

详细命令见 [候选包安装与独立编程](release-installation.md)。

## 5. 维护者候选发布流程

下面命令都在完整源码的 Linux 环境执行。路径中的时间戳只用于隔离本次证据；不要复用旧证据目录。

```bash
source scripts/env.sh
export CARGO_INCREMENTAL=0
export RC_ROOT="$PWD/artifacts/rc-0.1.0-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$RC_ROOT"

bash scripts/release/check.sh

python3 scripts/release/dependency_inventory.py \
  --output "$RC_ROOT/third-party"
export DMS_THIRD_PARTY_DIR="$RC_ROOT/third-party"

python3 scripts/package_sdk.py \
  --evidence-dir "$RC_ROOT/evidence/rust-sdk"

python3 scripts/sdk/package_go.py \
  --output "$RC_ROOT/go-proxy"

./scripts/build.sh
./scripts/package.sh "$RC_ROOT/server"
```

`package_sdk.py` 最后一行会打印 Rust SDK crate 路径；`package.sh` 会打印 server tar 路径。把它们放入总清单：

```bash
export SDK_CRATE='<package_sdk.py 输出的 dms-client-0.1.0.crate>'
export SERVER_TAR='<package.sh 输出的 dms-server-0.1.0-linux-*.tar.gz>'

python3 scripts/release/candidate_manifest.py \
  --rust-sdk-crate "$SDK_CRATE" \
  --go-proxy "$RC_ROOT/go-proxy" \
  --server-archive "$SERVER_TAR" \
  --third-party-inventory "$RC_ROOT/third-party/inventory.json" \
  --output "$RC_ROOT/release-candidate-manifest.json"
```

隔离消费者验收单独执行，不能用源码单测替代：

```bash
bash scripts/release/accept.sh \
  --sdk-crate "$SDK_CRATE" \
  --server-archive "$SERVER_TAR" \
  --output-dir "$RC_ROOT/evidence/isolated-consumer"

python3 scripts/release/candidate_manifest.py \
  --rust-sdk-crate "$SDK_CRATE" \
  --go-proxy "$RC_ROOT/go-proxy" \
  --server-archive "$SERVER_TAR" \
  --third-party-inventory "$RC_ROOT/third-party/inventory.json" \
  --acceptance-result "$RC_ROOT/evidence/isolated-consumer/results/consumer-result.json" \
  --output "$RC_ROOT/release-candidate-manifest.json"
```

`release-candidate-manifest.json` 是本次候选版本的总索引，至少要记录：版本、源码 commit、各制品路径、字节数、SHA256、Rust/Go/Server 角色和隔离验收结果。它仍然声明 `remote_publish=false`。Go SDK 若来自本地 module proxy，只代表该 proxy artifact；不要把未提交工作区能力写成公开源码 pin。

## 6. 发布前检查清单

- [ ] `git status --short` 干净，当前 commit 是要发布的源码。
- [ ] `scripts/release/check.sh` 在 Linux 通过。
- [ ] Rust SDK `.crate` 生成并通过独立消费者编译。
- [ ] Go SDK proxy 生成；若本次不公开 Go，则在总清单中保持 preview。
- [ ] Server tar 生成；包内 `SHA256SUMS` 和外部 `.tar.gz.sha256` 均可校验。
- [ ] 第三方依赖清单生成；缺失许可证文本时不能忽略。
- [ ] `scripts/release/accept.sh` 在不挂载源码、不安装 protoc 的容器内通过。
- [ ] 单 VM 手动 SET/GET、3 VM 跨 Node 读、观测栈验证按需要补充证据。
- [ ] Go SDK 若对外给 source pin，确认对应 commit/tag 已包含本页声明接口；本地 RC proxy 另记录 archive SHA。
- [ ] 未创建 tag、GitHub Release 或远端 registry 发布，除非已有单独授权。
