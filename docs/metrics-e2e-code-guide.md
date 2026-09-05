# Metrics 代码导读

先用[单VM手册](metrics-single-node-manual.md)看到真实数值，再按下面路径阅读。此页描述当前代码责任，不用历史运行的固定指标数或哈希代替验收。

## 1. 一个 GET 如何变成图上的数值

```text
宿主创建 Registry → 注入 ClientOptions
  → ClientMetrics 注册 Collector 并保存句柄
  → GET开始：begin_operation，inflight+1，记录Instant
  → GET结束：Guard Drop，inflight-1，次数+1，耗时observe
  → 宿主HTTP调用 encode_text(Registry)
  → Prometheus定期抓取 → Grafana查询
```

Collector是“能提供一组指标数据”的对象；Registry是这些对象的登记表，不是网络客户端。register建立指标和Registry的关联，返回的Metrics对象保存共享句柄，业务调用只更新句柄指向的数值。

同一Registry内多个Client复用已注册typed bundle，避免重复名称注册；不同进程有自己的Registry，Prometheus通过target的job/instance区分。

## 2. 按业务责任看指标

| 所有者 | 主要观察什么 | 定义/更新入口 |
| --- | --- | --- |
| Client | 用户操作次数/耗时/inflight、缓存、payload、会话、mapping | [Client metrics](../sdk/rust/dms-client/src/metrics.rs) |
| Node | Arena分配/隔离字节、staging、FD、Peer传输、命令处理 | [Node metrics](../server/src/node/metrics.rs) |
| Meta | 当前状态量、提交/解析、journal/checkpoint、lease/watch | [Meta metrics](../server/src/meta/metrics.rs) |
| RPC双端 | 实际Client调用、Server处理次数与耗时 | [RpcMetrics](../common/metrics/src/lib.rs) |
| Trace runtime | 导出批次、错误、丢弃、队列 | [TraceRuntimeMetrics](../common/metrics/src/lib.rs) |

没有真实RDMA/UB/L2功能就不能为了面板好看制造指标值。空闲时Counter/Gauge可以为零，未实现功能与真实零负载不是一个概念。

## 3. 找到网络出口

| 步骤 | 代码 |
| --- | --- |
| Registry、Collector注册、`encode_text` | [common/metrics](../common/metrics/src/lib.rs) |
| Node/Meta status HTTP 路由 | [服务status](../server/src/health.rs) |
| SDK宿主自己的HTTP示例 | [metrics_host](../sdk/rust/dms-client/examples/metrics_host.rs) |
| 抓取周期与目标 | [prometheus.yml](../infra/observability/prometheus/prometheus.yml)、[file_sd](../infra/observability/prometheus/file_sd/targets.json) |
| 展示配置 | [Grafana目录](../infra/observability/grafana) |

`/metrics`每次返回累计快照，不负责重置Counter。Prometheus把各时刻快照存成时间序列，`rate()`根据时间差计算速率。

## 4. 阅读规则

用户操作耗时、RPC耗时、payload耗时是不同口径，不能相加当总时间；同一个请求可以同时贡献这几类指标。Meta全状态快照按周期采样，避免每次小请求遍历整个状态，但周期快照成本仍随状态量增长。

具体封装规范见[typed Metrics说明](metrics-typed-api-e2e-guide.md)。错误、日志和Trace与Metrics保持独立；Trace ID仅作为Exemplar关联，不进入普通label。
