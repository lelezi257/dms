# 配置：谁读取、何时生效

不要混淆三种入口：SDK 参数由应用读取，Node/Meta 配置由服务进程读取，`DMS_*` 实验部署变量由脚本转换成启动参数。

## 1. Rust SDK

优先级是 **默认值 < 环境变量 < 显式 ClientOptions**，连接创建时解析。修改 shell 环境不会在线改变已存在的 Client。

```rust
use dms_client::{ClientOptions, DmsClient};
use std::time::Duration;

let client = DmsClient::connect("http://127.0.0.1:25200", ClientOptions {
    timeout: Some(Duration::from_secs(5)),
    current_cache_bytes: Some(32 * 1024 * 1024),
    ..ClientOptions::default()
})?;
```

| ClientOptions 字段 | 环境变量 | 默认值 / 限制 |
| --- | --- | --- |
| `endpoint` | `DMS_ENDPOINT` | 必须提供；`connect(endpoint, ...)` 的地址为显式值 |
| `timeout` | `DMS_TIMEOUT_MILLIS` | 30000 ms，正数 |
| `inline_threshold_bytes` | `DMS_INLINE_THRESHOLD_BYTES` | 65536，正数；控制小 value 的 SET 请求内联，以及非 SHM 单 GET 响应内联预算；GET 另受协议 64 KiB 上限约束 |
| `current_cache_bytes` | `DMS_CURRENT_CACHE_BYTES` | 64 MiB；0 关闭；不是进程 RSS 上限 |
| `heartbeat_interval` | `DMS_HEARTBEAT_INTERVAL_MILLIS` | 10000 ms；缓存续租会按 Node 租约缩短 |
| `session_channel_capacity` | `DMS_SESSION_CHANNEL_CAPACITY` | 64，正数 |
| `shared_memory` | `DMS_SHARED_MEMORY` | false；true 只在支持 SHM 的本地 UDS 会话生效 |
| `default_durability` | `DMS_DEFAULT_DURABILITY` | `LocalMemory`；其它枚举存在不代表后端已实现 |
| `tls` | `DMS_TLS_MODE` | 当前仅 `Disabled`；不是完整 TLS 用户配置 |
| `metrics_registry` | 无 | 宿主注入；SDK 不打开 HTTP 端口 |

SHM 模式不保留另一份 owned Current cache；TCP owned cache 需同时满足版本、generation 和租约条件。预算按 key、payload 与固定条目开销计费，超预算整批淘汰，不是 LRU。

SDK 不安装宿主日志后端或 Subscriber。示例程序读取 `DMS_TRACING_*` 是**宿主示例**的能力，不应当作 DmsClient 的通用环境配置。

## 2. Node / Meta：TOML 与启动参数

优先级是 **默认值 < `--config` 指定的 TOML < CLI**。不自动扫描配置文件、不 watch 文件变化；没有配置路径就只有默认值和 CLI。

例如把以下内容保存为 `.local/manual/node.toml`：

```toml
node_id = "manual-node"
health_address = "0.0.0.0:25000"
worker_tcp_address = "127.0.0.1:25200"
meta_endpoint = "http://127.0.0.1:25300"
arena_capacity_bytes = 1073741824
region_size_bytes = 67108864
staging_ttl_millis = 30000
client_cache_lease_ttl_millis = 1000

[log]
level = "info"
format = "json"
path = ".local/manual/log/dms-node.log"
overflow = "drop-and-report"
max_file_size = "256MiB"
max_backups = 14
max_age = "7d"

[tracing]
enabled = false
periodic_operations = false
otlp_endpoint = "http://127.0.0.1:4317"
sample_ratio = 0.01
```

从源码根目录运行，CLI 的 debug 覆盖文件中的 info：

```bash
"$CARGO_TARGET_DIR/release/dms-node" serve \
  --config .local/manual/node.toml --log-level debug
```

| 范围 | 关键字段和默认 |
| --- | --- |
| Node 必需 | `node_id`、`meta_endpoint`；TCP/UDS listener 至少一个 |
| Node 内存 | `arena_capacity_bytes=1 GiB`，`region_size_bytes=64 MiB`，`staging_ttl_millis=30000` |
| Node 缓存租约 | `client_cache_lease_ttl_millis=1000`，范围 1..=30000；实际授予还受 Meta 剩余期限限制 |
| Meta 必需 | `node_id`、`grpc_address` |
| Meta journal | 不指定 `journal_dir` 则用内存后端；指定目录才使用 WAL/snapshot |
| Meta checkpoint | `checkpoint_every_records=4096`，正数；CLI 为 `--checkpoint-every-records` |
| status | 未指定 `health_address` 时为 `127.0.0.1:0`（随机空闲端口）；手工部署建议显式固定 |
| 日志 | info、JSON、stderr；配置 path 才写文件；异步队列10240、DropAndReport、256 MiB滚动、14备份、7天老化 |
| Trace | 默认关闭；启用后的默认采样0.01；周期成功请求默认不采集；队列4096、批次512、间隔5000ms、导出超时3000ms |

Meta 使用同样的 `[log]`、`[tracing]` 配置。

`region_size_bytes` / `--region-size-bytes` 是新 Region 的扩容目标，不是一个 value 的大小。
多个 Slot 共用同一 Region/FD；优先使用现有空闲范围，再向 OS 申请。申请大于目标时
扩大该次 Region；预算尾部不足目标时按可用余额分配。参数至少 64 字节，向 64B 对齐，
重启生效。`resident_bytes` 表示 backing 预留容量，不等同于已经触碰的物理页/RSS。
已导出 Slot 的隔离仍占预算，大 Region 不代表完整 GC 已实现。

`checkpoint_every_records` 控制累计多少条 journal 记录后生成一次全量 snapshot，
不是 WAL 的刷盘间隔。WAL 模式仍然先可靠追加 journal、再 apply 状态，
不会因为降低 snapshot 频率而跳过已确认写入的恢复记录。
更大阈值减少全量复制/快照开销，但增加重启时重放的日志量；修改后重启生效。

完整可用 CLI 参数以当前二进制为准：

```bash
"$CARGO_TARGET_DIR/release/dms-node" serve --help
"$CARGO_TARGET_DIR/release/dms-meta" serve --help
```

## 3. 在线修改：当前只有内部能力，不是完整运维功能

| 配置 | 现有接线 | 用户现在怎么改 |
| --- | --- | --- |
| Node staging TTL | `NodeHandle::apply_config_change` → NodeState；影响后续 allocation | 尚无公开管理 HTTP/RPC/CLI，重新启动时配置 |
| 日志 level | logging handle 支持修改；Node 配置命令已接入 | 尚无公开管理入口，重新启动时配置 |
| listener / Meta endpoint / journal / Arena capacity / Region size | 标记为需要重启 | 重启；配置不能在线修改，运行中按已配置大小增加 Region |
| Client cache lease TTL | 仅启动配置 | 重启；不得缩短已有缓存的回收义务 |
| Meta checkpoint records | 仅启动配置 | 重启；不改变 journal 可靠性模式 |
| tracing 开关、采样、exporter | 启动初始化 | 重启；没有自动热重载 |

当前没有一个可供用户调用的通用 `/config` URL。定义了内部 controller 不等于已经支持在线运维。

## 4. 脚本变量不是服务进程的另一套优先级

`source scripts/env.sh` 设置开发默认目录、端口和日志/Trace变量；`deploy.sh` 把它们翻译成 `--...`。直接运行二进制不会自动读取这些部署变量。

| 使用方式 | 读取位置 | 注意 |
| --- | --- | --- |
| `deploy.sh` | 当前 shell 的 `DMS_*` | 仅重置实验；会清理 journal；Client metrics 固定19400 |
| 实验包 `cluster.sh` | `config/dms.env`，可由 `DMS_CONFIG_FILE` 改路径 | shell 文件，不是 Node/Meta TOML；配置文件中的赋值会覆盖同名已有变量 |
| `observability.sh` | Compose 的 `DMS_*` | 配置观测容器端口/镜像/日志目录，不配置 DMS业务 |

手工教程直接传 CLI，因此不会因为只 export 一个 `DMS_LOG_LEVEL` 就改变 Node 日志级别。要修改哪一层，先确定是谁读取它。

源码入口：[SDK配置](../sdk/rust/dms-client/src/client.rs)、[服务配置](../server/src/config.rs)、[开发环境脚本](../scripts/env.sh)、[实验部署脚本](../scripts/deploy.sh)。
