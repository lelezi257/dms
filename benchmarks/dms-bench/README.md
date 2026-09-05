# dms-bench

进程级 Rust SDK 基准，通过应用使用的同一 endpoint 访问 `dms-node`。输出
`dms.benchmark.v2` JSON；保留九个案例名、预热及三轮计时结构。

| 案例 | 一次计时操作 |
| --- | --- |
| `set_inline` | 小对象 SET |
| `set_staged_grpc` | 大对象 SET；保留历史名称，实际 transport 由 endpoint 决定 |
| `get_current_uncached` | 新建 reader 后 GET，连接成本包含在计时内 |
| `set_range` | 对已有对象应用中间补丁 |
| `mset` / `mget` | 一次批量请求，包含 batch_size 个对象 |
| `hset` / `hget` / `hscan` | 字段写入、单字段读取或第一页有序扫描 |

每个样本均校验精确 bytes、长度、存在性；Hash 扫描额外校验字段顺序、
数量与是否还有下一页。写操作完成后的完整回读在主操作计时外；GET/HGET/
MGET/HSCAN 保留读取结果校验在计时内。准备、预热、操作或校验错误使进程
返回非零，原始 JSON 仍写出。预热错误独立记录，不冒充计时样本。

`samples` 是实际计时操作数；`attempted_operations` 包括连接/准备失败的尝试。
每个主操作是一条逻辑 SDK 请求，批量对象/字段数见 `items_per_operation`，
不可将它当成 RPC 次数。`throughput_ops_per_sec` 的分母仅为主操作累计时间，
不是整个进程墙钟吞吐；存在错误的报告不可作为成功吞吐。

RPC 次数和 payload copy 未做插桩，保留兼容字段但值为 `null`，并标记
`not_instrumented`。不再从过时日志文本推导 metrics；外层审计 runner
抓取真实 `/metrics` 前后样本，并明确其包含准备、预热与完整数据校验。

进程资源取自 Linux `/proc`。Client 使用当前 PID；Node/Meta 只接受调用方
显式传入的 `DMS_BENCH_NODE_PID` / `DMS_BENCH_META_PID`。未指定 PID 的资源
为 `null`，不从遗留部署文件猜测。RSS 是前后快照而非峰值，CPU ticks
属于整轮基准（含校验），不能当作单操作 CPU。

本基准不模拟 RDMA/UB，不证明同步多副本、落盘或对象存储 durability；
单 VM 小样本只用于本项目同配置诊断对照，不代表生产吞吐。
