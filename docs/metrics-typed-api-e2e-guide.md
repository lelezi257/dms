# Typed Metrics：为什么有 Guard、record 和 set

本页是[Metrics代码导读](metrics-e2e-code-guide.md)的实现补充。不要再给每个指标发明一种调用方式；现有模式按“何时知道数值”分三类。

| 方式 | 适用场景 | 为什么封装 |
| --- | --- | --- |
| `begin_xxx() -> Guard` | 开始时不知道耗时，结束时才知道 | 统一成对维护inflight、次数和耗时，错误提前返回也能Drop |
| `record_xxx(...)` | 一个事件已完成，结果/字节数/耗时已知 | 一次更新属于同一完成事件的多个Collector |
| `set_xxx(value)` | 当前状态快照 | 表达当前量，不误当累计次数 |

例如一次传输完成，record可以同时更新“传输了1次”“传输了4096bytes”“耗时0.2ms”，它们是同一事件的三个维度，不是重复记录三次传输。

## 1. Guard 不是后台任务

```text
let guard = begin_operation(Get)
  保存开始Instant；inflight+1
执行业务
  成功时标记success；未标记默认error
guard离开作用域
  Drop读取elapsed；inflight-1；Counter+1；Histogram.observe(elapsed)
```

耗时被加到 `dms_client_operation_duration_seconds` 对应操作标签的Histogram，不是加到某一条日志。`observe`更新sum/count/buckets，不保留无限增长的每请求列表。Rust `?`提前返回也会Drop局部Guard；但强制杀进程不能保证最后一笔指标可见。

RPC使用对称的 `begin_client_call` / `begin_server_call`，Client计真实网络attempt；长流这里主要计建流，Session/Watch的长期存活由业务指标表达。

## 2. 为什么标签要封装

业务传入枚举或闭集方法，`metrics.rs`负责转换成固定标签。这样避免把key/endpoint/任意错误message误塞入label，避免拼写不同产生两组指标，也能统一结果分类；不只是少写一个字符串。

Collector字段不对业务公开。已知Counter简单也应通过所属Metrics的方法更新，不在各处调用`with_label_values`。

## 3. 关键代码

- [公共Metrics](../common/metrics/src/lib.rs)：Registry共享注册、RPC Guards、Exemplar、输出编码。
- [ClientMetrics](../sdk/rust/dms-client/src/metrics.rs)：OperationGuard、事件record、Session生命周期。
- [NodeMetrics](../server/src/node/metrics.rs)：Arena/Peer/FD/命令的有限标签。
- [MetaMetrics](../server/src/meta/metrics.rs)：Meta业务事件与状态快照。

指标与业务边界一起review：谁开始、谁结束、失败如何标记、是否可能重复计数、是否有高基数输入。不要用已经结束的Guard再手工补一次同名Histogram。
