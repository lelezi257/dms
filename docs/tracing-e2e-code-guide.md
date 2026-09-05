# Trace 代码导读

先按[单 VM SET/GET教程](tracing-single-node-manual.md)看到一次请求，再看这条代码链。日志仍由日志系统落盘；这里的Span用于请求结构和耗时。

## 1. 一条 GET 跨三个进程

```text
SDK宿主创建根Span
  → DmsClient GET子Span
  → gRPC Client向metadata注入traceparent
  → Node Server提取父上下文，创建请求Span
  → 命令envelope带TraceContext，NodeState继续子Span
  → 调用Meta时再次注入metadata
  → Meta Server/命令处理接上同一Trace
  → payload download建Span；用户bytes不改变
各Span结束 → 各进程batch exporter → Alloy → Tempo汇总
```

普通业务API不增加trace_id参数。Proto用户数据不承载trace header；gRPC metadata是现有请求头通道。共享内存或网络payload本身仍有耗时Span，只是不把Trace信息写入用户value。

## 2. 文件与抽象

| 文件 | 职责 |
| --- | --- |
| [config.rs](../common/tracing/src/config.rs) | 开关、采样、队列、OTLP地址和进程身份 |
| [init.rs](../common/tracing/src/init.rs) | Subscriber、OpenTelemetry Layer、Exporter、退出flush |
| [context.rs](../common/tracing/src/context.rs) | 捕获/设置父上下文、日志correlation、Metric Exemplar |
| [grpc.rs](../common/tracing/src/grpc.rs) | metadata注入/提取、Client interceptor、已知RPC命名 |
| [server.rs](../common/tracing/src/server.rs) | Server layer包装请求处理；关闭时短路 |
| [lib.rs](../common/tracing/src/lib.rs) | 统一导出既有工具，不发明第二套Span生命周期 |

`GrpcServerTraceLayer`是Tower/Tonic服务组合的适配，不是Proto自动生成的业务handler。业务handler仍处理SET/GET；layer在其外层建立请求上下文。

## 3. 谁结束 Span

异步代码用`.instrument(span)`在Future被poll时进入Span；Future完成并释放最后一个Span句柄后关闭Span。同步短代码可用scope guard；不要把同步enter guard持有到跨线程await。

不是每段业务都要套新的`async {}`：可以给已有Future加instrument，或在函数边界用合适的instrument属性。只在核心耗时点打Span，不逐函数扩张。

Span结束后SDK交给配置好的batch exporter，可能稍后与其它Span一起经OTLP发到Alloy；不是最后一个RPC把整棵树打包。日志关联ID在当前上下文有效时提取，异步写线程不负责建立父子关系。

## 4. 默认行为与排障

Node/Meta默认Trace关闭；SDK不接管宿主Subscriber。默认关闭成功周期心跳Trace并保留Metrics。启用后用有界、具体操作名，不能把key/ID拼入Span名。

未知TraceID、采样未中、exporter未flush、Tempo时间范围、缓存命中省略远端步骤，都可能导致“看不到预期Span”，应分别核查。历史固定Span数/吞吐数字不是当前协议合同，正确性看同一个真实请求的父子关系、服务身份和业务结果。

关键业务入口：[SDK示例](../sdk/rust/dms-client/examples/sdk_kv.rs)、[Client内部](../sdk/rust/dms-client/src/internal/client_impl.rs)、[Node状态机](../server/src/node/runtime.rs)、[Meta状态机](../server/src/meta/runtime.rs)。
