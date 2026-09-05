# 单 VM Metrics 手工验证

先完成[单 VM 手工教程](local-single-vm-manual.md)，保留 Meta/Node 前台进程；本页沿用 Node status25000、Meta status25100、Worker25200。不要直接运行 `metrics-targets.sh single`：它为旧默认实验端口19000/19100/19400生成 targets，与本教程不同。

## 1. 启动一个提供 Registry 的应用

新 VM 终端，在源码根目录：

```bash
source scripts/env.sh
export DMS_ENDPOINT="unix://$PWD/.local/manual/run/worker.sock"
export DMS_CLIENT_METRICS_ADDRESS=0.0.0.0:25400
"$CARGO_TARGET_DIR/release/examples/metrics_host"
```

这个宿主创建 Registry，注入 SDK，执行普通和较大 SHM SET/GET，并提供 `/metrics`。SDK 本身不启动 HTTP server。保留终端；此示例主要产生启动时的测试数据，不应期待它持续生成业务 QPS。

## 2. 不依赖 Prometheus，先看原始输出

另一个终端：

```bash
curl -fsS http://127.0.0.1:25000/metrics
curl -fsS http://127.0.0.1:25100/metrics
curl -fsS http://127.0.0.1:25400/metrics
```

应看到 `# HELP`、`# TYPE` 和数值。Counter累计事件次数，Gauge表示当前状态，Histogram累积耗时分布。`duration_seconds_sum` 是累计秒数，不是最近一次请求耗时。

## 3. 配置抓取目标

使用编辑器把 `infra/observability/prometheus/file_sd/targets.json` 设置为以下内容；这是观测栈的共享配置，**只在专属于此教程的栈上替换**：

```json
[
  {"targets":["host.docker.internal:25000"],"labels":{"job":"dms-node","instance":"manual-node","component":"node"}},
  {"targets":["host.docker.internal:25100"],"labels":{"job":"dms-meta","instance":"manual-meta","component":"meta"}},
  {"targets":["host.docker.internal:25400"],"labels":{"job":"dms-client","instance":"manual-client","component":"client"}}
]
```

`host.docker.internal` 在此 Compose 中映射到 **Linux VM 宿主**，不是 Mac。status listener 使用0.0.0.0才允许容器访问。Prometheus file_sd 会自动重新读取目标文件，不需要重启 Node。

按[观测栈启动说明](observability.md)启动容器，随后执行：

```bash
curl -fsS http://127.0.0.1:9090/api/v1/targets
```

浏览器打开 `http://VM地址:9090/targets`，三项都应为 UP。然后打开 `http://VM地址:3000`，登录并查看 `DMS Overview`。刚启动需要等待抓取周期；若只做少量启动请求，累计 counter 比5分钟rate更直观。

## 4. 看什么

在 Prometheus 或 Grafana Explore 的 `DMS Prometheus` 中查询：

```promql
dms_client_operations_total
dms_node_arena_quarantined_bytes
up{job=~"dms-node|dms-meta|dms-client"}
```

前者应包含已完成SET/GET；隔离字节数用于观察SHM尚不能安全复用的内存，不能当作“已回收”字节。详细指标命名和更新位置见[Metrics代码导读](metrics-e2e-code-guide.md)。

实验脚本 `verify_metrics.sh` 的默认端口与本页不同，不要直接运行并把其它进程的结果当成本教程验收。这里以三个实际端口和对应 Prometheus targets 为准。退出时仅在 metrics_host 自己的终端 Ctrl-C。
