# 日志代码导读

运行入口见[观测说明](observability.md)。Node/Meta负责日志进程初始化；SDK只调用宿主日志接口，不私自创建文件或线程。

## 1. 一条业务日志怎么下盘

```text
业务 info!/warn!/error!
  → 先检查level，再读取当前Trace关联字段
  → slog结构化记录
  → 有界异步队列
  → JSON或Text formatter
  → RotatingFileWriter（或stderr）
  → 本机文件
  → Alloy读取 → Loki → Grafana
```

formatter接收实现 `std::io::Write` 的writer，因此文件滚动只需在writer中处理，不让业务知道文件名或rename。队列满默认DropAndReport，保护业务不被日志磁盘反压；这不是无损审计日志。

## 2. 文件与责任

| 文件 | 看什么 |
| --- | --- |
| [lib.rs](../common/logging/src/lib.rs) | 对外宏、level短路、调用现场关联trace_id/span_id |
| [config.rs](../common/logging/src/config.rs) | level、formatter、输出、滚动、保留和队列策略 |
| [init.rs](../common/logging/src/init.rs) | Drain/writer组装、进程初始化、LoggingGuard退出flush、level controller |
| [file_writer.rs](../common/logging/src/file_writer.rs) | 实现Write，达到滚动条件时切换文件 |
| [retention.rs](../common/logging/src/retention.rs) | 历史日志数量/年龄清理；不管理业务WAL |
| [fallback.rs](../common/logging/src/fallback.rs) | 初始化/写入异常走stderr，避免递归记日志 |

`LoggingGuard`必须活到进程退出；过早Drop会提前结束日志生命周期。内存中的level controller可以改过滤级别，但当前并没有对外通用HTTP配置入口，见[配置说明](configuration.md)。

## 3. 调用形式

```rust
use dms_logging::warn;

warn!(
    "request failed";
    "event" => "node.request.failed",
    "error_code" => code,
);
```

这是宏支持的语法：分号前是文本，后面是结构化字段；`=>`不是普通函数参数。message解释事件，字段支持机器检索。value和用户敏感内容不应该直接进入日志。

SDK里的 `log::warn!` 由宿主安装的 `log` 后端接收；例如宿主选择env_logger就由它输出。SDK不会安装Node的slog全局writer。

## 4. Review重点

默认info不要逐请求倾倒日志；昂贵字段应在level检查后求值。trace关联在业务调用现场取得，不让异步写线程猜当前请求。文件滚动/老化只属于日志目录，绝不删Meta journal来“清理日志”。
