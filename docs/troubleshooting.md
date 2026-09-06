# 排障：先定位进程，再定位一次请求

以下单VM地址沿用[手工教程](local-single-vm-manual.md)。使用其它端口时同步替换，不要只看到一个健康响应就假定来自刚启动的进程。

## 1. 启动与连接

| 现象 | 先检查 | 不要做什么 |
| --- | --- | --- |
| `dms-dev`不存在 | 源码根 `./scripts/vm.sh up`；不要照抄未创建的n1名字 | 不必改业务源码 |
| Cargo磁盘不足 | VM内 `df -h`、`du -sh "$CARGO_TARGET_DIR"`，确认是否编译缓存占用 | 不删除journal/用户数据来腾空间 |
| Address already in use | VM内 `ss -ltnp`，核对端口和PID；换空闲端口 | 不直接杀未知PID |
| Node启动失败 | Meta `:25100/readyz`、Meta端点是否为业务RPC25300 | status端口不是gRPC端口 |
| TCP可用但UDS失败 | 路径、目录权限、连接者是否同一Linux内核、FD Broker是否存在 | 不能拿远端VM的文件路径当本机UDS |
| 远端GET失败 | Node注册的TCP地址是否可达；不能注册0.0.0.0/回环作远端位置 | 不先猜Meta丢key |

```bash
curl -fsS http://127.0.0.1:25100/readyz
curl -fsS http://127.0.0.1:25000/readyz
tail -n 50 .local/manual/log/dms-node.log
tail -n 50 .local/manual/log/dms-meta.log
```

启动日志配置失败时消息可能只在终端stderr；使用实验部署脚本则看对应 `*.fallback.log`。

## 2. 读写结果

- `get()` 返回 `Ok(None)` 表示key不存在；网络超时返回 `Err(DmsError)`，不是同一含义。根据数字ErrorCode常量处理，message用于解释这次失败，不做字符串匹配业务分支。
- SET返回成功后无需sleep才能读取；但SET本身可能等缓存失效ACK/旧租约到期。当前部分写入尾延迟仍可能明显大于中位数。
- 超时不总等于服务端没有提交。保留同一逻辑请求的operation ID进行安全重试；不要把所有失败都当成“可删除新Block”。
- Meta恢复journal不代表value恢复。LocalMemory写依赖Node内存；Node重启后元数据可能还在但唯一payload已丢失。
- SHM容量耗尽可能来自已导出allocation的隔离。它是当前保守生命周期边界，不是改TTL就可以强制复用；完整GC未完成。
- `NODE_ARENA_CAPACITY_EXHAUSTED (0x02010001)` 表示配置的 Arena/Group 预算不足；`NODE_ARENA_ALLOCATION_FAILED (0x02010006)` 表示 OS backing 创建失败。后者先看 Node 日志和该进程的 `/proc/<pid>/limits`、FD 数量、系统内存，不要误判为 key/value 参数错误。默认大 Region 降低 FD 增长，但不替代长期回收。

## 3. Metrics没有数据

先curl进程`/metrics`，再查Prometheus `/targets`。原始endpoint有值而target为DOWN，多半是容器访问地址/端口/监听范围不匹配。单VM新教程用25000/25100/25400，旧默认脚本用19000/19100/19400，不能混用。

SDK不会自己暴露Metrics；宿主需注入Registry并持续提供HTTP。刚执行即退出的sdk_kv不能作为Prometheus长期target。

## 4. Trace没查到或看不懂

检查顺序：

1. Client宿主、Node、Meta是否都启用Trace，验收采样是否1.0。
2. 是否使用本次输出的真实ID，而不是文档里的历史ID。
3. Alloy4317是否可达，`./scripts/observability.sh status/logs`是否正常。
4. 等数秒让服务端batch导出，再调用Tempo API；404不同于业务GET失败。
5. 查看根名`dms.sdk_kv.set/get`，不要把其它旧Trace或周期诊断请求当成本次请求。

Trace包含多个Span，所以不是每行一个请求。缓存命中可能没有Meta/Peer子Span；跨Node首次冷读才适合验证完整数据路径。

## 5. 日志关联为空

先确认JSON日志文件有新增，再确认Alloy挂载了相同目录。请求级日志可能需debug；没有请求日志不等于Trace丢失。Loki标签只含有限进程信息，trace_id在JSON字段中；查询应先选择日志流，再解析JSON过滤。

## 6. 安全恢复原则

保留错误码、时间、节点、日志和真实Trace ID后再重启。普通排障不清理WAL，不使用`deploy.sh`重置有价值数据，不删除观测卷。无法确认数据位置或提交状态时，先把它作为不确定结果报告，而不是输出成功。

相关入口：[配置](configuration.md)、[观测部署](observability.md)、[单VM教程](local-single-vm-manual.md)。
