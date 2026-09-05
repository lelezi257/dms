# 观测：Metrics、日志、Trace 各管什么

| 问题 | 数据 | 采集与查询链 |
| --- | --- | --- |
| 最近请求量、错误率、p99 如何？ | Metrics | 进程 Registry → `/metrics` → Prometheus → Grafana |
| 这次失败为什么发生？ | 日志 | Node/Meta JSON 文件 → Alloy → Loki → Grafana |
| 一次 GET 时间花在哪？ | Trace | Client/Node/Meta Span → OTLP → Alloy → Tempo → Grafana |

这些是旁路观测系统，不参与 value 提交。Tempo 保存调用链，不是替代 Prometheus；Alloy 同时可以读取日志文件和接收 OTLP。

## 1. Linux 内启动观测栈

先完成[单 VM 教程](local-single-vm-manual.md)。Docker 也安装在 VM 内，不使用 Mac Docker 来运行这些进程。Ubuntu 首次安装：

```bash
sudo apt-get update
sudo apt-get install -y docker.io docker-compose-v2
sudo systemctl enable --now docker
```

在已加载 `scripts/env.sh` 的源码终端：

```bash
export DMS_RUNTIME_ROOT="$PWD/.local/manual"
export DMS_ALLOY_INSTANCE=manual-vm
./scripts/observability.sh up
./scripts/observability.sh status
```

开发脚本根据 `DMS_RUNTIME_ROOT/log` 挂载 Node/Meta 日志。若在另一目录运行实验包，则直接设置 `DMS_LOG_DIR`。不要将已运行观测栈的挂载目录悄悄切换到另一组服务。

首次镜像下载依赖网络。镜像名可通过 `DMS_PROMETHEUS_IMAGE`、`DMS_GRAFANA_IMAGE`、`DMS_LOKI_IMAGE`、`DMS_ALLOY_IMAGE`、`DMS_TEMPO_IMAGE` 覆盖。Tempo 拉取受限时有 `prepare-tempo-image` 备用构建命令，它下载官方发布二进制并构建本地镜像；不是离线安装器。

## 2. 端口与页面

| 端口 | 使用者 / 地址 |
| --- | --- |
| 3000 | Grafana，默认开发账户 `admin` / `dms-dev` |
| 9090 | Prometheus，`/targets` 查看抓取状态 |
| 3100 | Loki API，不是独立图形界面 |
| 3200 | Tempo API，`/api/traces/<trace_id>` 查询链路 |
| 12345 | Alloy 状态界面 |
| 4317 / 4318 | Alloy OTLP gRPC / HTTP 接收端，不是 Grafana 页面 |

在 VM 本机可用 `curl`；浏览器使用可访问的 VM 地址加端口，Lima 自动转发可用时也可使用 Mac `localhost`。不能把“VM 内 127.0.0.1”无条件当成“Mac 内 127.0.0.1”。这些端口默认不带生产认证，不应暴露公网。

Grafana 已通过 provisioning 装载 `DMS Prometheus`、`DMS Loki`、`DMS Tempo` 数据源及 `DMS Overview` 仪表盘。

## 3. 查询示例

Metrics：先按照[单节点 Metrics](metrics-single-node-manual.md)配置正确 targets。Prometheus 会给每个进程附加 `job/instance`，集群总量由查询聚合，而不是由 Node 互相上报合并。

```promql
sum(rate(dms_client_operations_total[5m]))
sum by (instance) (rate(dms_node_replica_bytes_total{direction="receive"}[5m]))
histogram_quantile(0.99, sum by (le) (rate(dms_client_operation_duration_seconds_bucket[5m])))
```

短命令退出后不再提供 SDK Registry；要持续抓取 Client 指标，应使用宿主 HTTP endpoint，例如 `metrics_host`。不能把多个节点的 p99 直接相加。

日志：Grafana → Explore → `DMS Loki`，先查：

```logql
{job="dms"}
```

再按 `service_name/instance/level` 缩小范围。查一个 Trace 的相关日志：

```logql
{job="dms"} | json | trace_id="替换为实际trace_id"
```

Trace：见[单 VM Trace 教程](tracing-single-node-manual.md)。展开树看到操作名，横条是耗时；父 Span 包含子 Span，因此不能把所有横条时长直接相加。

## 4. 三者如何关联

日志在调用现场自动读取当前 Span 的 `trace_id/span_id`，异步 writer 只负责下盘，不重新查上下文。Trace 页面可以跳到 Loki；前提是该时间段确有业务日志，info 默认不保证逐请求都有日志。

Metrics 的 Histogram Exemplar 保存少量 sampled Trace 的关联信息，`trace_id` 不是普通 label。高基数的 key、object ID、operation ID 不进入 Metrics label；否则每个新对象都会创建新时间序列。

## 5. 默认开销与限制

- Trace 默认关闭；临时验收可采样1.0，正常启用可降低采样。健康 Heartbeat/KeepAlive 默认保留 Metrics 而不导出成功周期 Span。
- 关闭 Trace 不是停止所有业务计时；Metrics 仍有更新开销，SDK有宿主 Subscriber 时按宿主配置工作。
- 日志默认 info、异步 DropAndReport；队列满可能丢日志并报告，不能当审计级无损日志。
- 当前 Compose 是开发单中心栈，不是生产 HA/备份方案。停止用 `./scripts/observability.sh down`；默认保留命名数据卷，不使用 `down -v` 做普通停止。
- 服务原生配置、脚本变量和在线能力见[配置说明](configuration.md)。

代码入口：[观测 Compose](../infra/observability/compose.yaml)、[Alloy](../infra/observability/alloy/config.alloy)、[Grafana配置](../infra/observability/grafana/provisioning)、[Prometheus配置](../infra/observability/prometheus/prometheus.yml)。
