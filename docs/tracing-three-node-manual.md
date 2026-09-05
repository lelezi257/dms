# 三 VM Trace：看到远端 Peer 读取

先按[三 VM 部署](metrics-three-node-manual.md)准备n1 Meta、三台Node与n1观测栈。本页IP仍是占位示例，必须替换成实际地址。

## 1. Trace 统一导出到 n1 Alloy

三台运行包 `config/dms.env` 修改：

```text
DMS_TRACING_ENABLED=true
DMS_TRACING_SAMPLE_RATIO=1.0
DMS_TRACING_OTLP_ENDPOINT=http://192.168.104.4:4317
DMS_LOG_LEVEL=debug
```

配置只在启动时读取。尚未启动则按三VM教程顺序启动；已运行则只重启这些实验进程，不清理journal。不要以为改文件本身会热更新。

## 2. n2 / n3 可选采集日志

在n2安装Docker后，在运行包目录：

```bash
export DMS_ALLOY_INSTANCE=n2
export DMS_LOKI_URL=http://192.168.104.4:3100/loki/api/v1/push
export DMS_LOG_DIR="$PWD/log"
./scripts/observability.sh agent-up
```

n3把instance改成n3。这里本机Alloy用于读日志，DMS进程的OTLP按配置直接发送n1；不要把本机Alloy当成已配置到远端Tempo的通用Trace代理。

## 3. 从 n1 写，在 n2 第一次读

在n1运行包目录：

```bash
export DMS_ENDPOINT=http://192.168.104.4:19200
export DMS_TRACING_ENABLED=true
export DMS_TRACING_SAMPLE_RATIO=1.0
export DMS_TRACING_OTLP_ENDPOINT=http://192.168.104.4:4317
export DMS_CLIENT_INSTANCE=manual-n1
./bin/sdk-kv set tutorial/peer-trace value-from-n1
```

在n2设置同样Trace变量，endpoint改成n2、instance改成manual-n2：

```bash
export DMS_ENDPOINT=http://192.168.104.5:19200
export DMS_TRACING_ENABLED=true
export DMS_TRACING_SAMPLE_RATIO=1.0
export DMS_TRACING_OTLP_ENDPOINT=http://192.168.104.4:4317
export DMS_CLIENT_INSTANCE=manual-n2
./bin/sdk-kv get tutorial/peer-trace value-from-n1
```

保存GET实际打印的Trace ID，按[单VM查询方法](tracing-single-node-manual.md)在n1 Grafana查询。第一次远端读预期包含n2 Client、n2 Node、n1 Meta、n1 Node的Peer传输。已经拉到本地后再次读可能不经过n1数据面；需要新key触发冷读，不要把命中解释为Trace缺失。

## 4. 验收与限制

展开对应Span的资源字段 `service.instance.id`，区分两个同名`dms-node`进程。Trace Context经gRPC metadata和内部命令传递；不放进value bytes，也不新增用户API的trace_id参数。

三台Exporter能导出并不自动证明它们属于同一Trace：要检查同一个实际GET的ID与父子链。三VM手册是操作路径，本次文档更新没有宣称重新完成整套三VM观测演练。结束后关闭100%采样、恢复info，仅停止自己启动的观测agent/实验进程。
