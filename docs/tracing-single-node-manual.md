# 单 VM：看到一次 SET / GET 的 Trace

沿用[单 VM 教程](local-single-vm-manual.md)的独立端口和目录；先按[观测栈说明](observability.md)启动 Alloy/Tempo/Grafana。

## 1. 给服务显式启用 Trace

在教程 Meta/Node 启动命令末尾都增加下列参数，再启动这两个教程进程。如果已在运行，只停止其前台进程后重启；不使用会清理journal的deploy脚本。

```text
--tracing-enabled true
--tracing-sample-ratio 1.0
--tracing-otlp-endpoint http://127.0.0.1:4317
--log-level debug
```

参数写在同一个shell命令中，换行需用 `\` 续行。这里1.0与debug只用于观察完整链路和日志关联。普通运行Trace默认关闭，日志info；成功周期心跳不会默认产生Trace。

## 2. SDK 宿主启用，再发请求

Client VM终端，在源码根目录：

```bash
source scripts/env.sh
export DMS_ENDPOINT=http://127.0.0.1:25200
export DMS_TRACING_ENABLED=true
export DMS_TRACING_SAMPLE_RATIO=1.0
export DMS_TRACING_OTLP_ENDPOINT=http://127.0.0.1:4317
export DMS_CLIENT_INSTANCE=manual-sdk
"$CARGO_TARGET_DIR/release/examples/sdk_kv" set tutorial/trace hello
"$CARGO_TARGET_DIR/release/examples/sdk_kv" get tutorial/trace hello
```

每条命令成功后分别打印一个 `trace_id=...`。SET与GET是两个请求，所以是两个不同ID；它们不是一个key对应的固定ID。`sdk_kv`作为宿主初始化Subscriber，SDK库不替用户安装全局Subscriber。

## 3. 在 Grafana 查刚才的请求

打开Grafana `:3000` → Explore → 数据源 `DMS Tempo`。

- 如果界面只有Search/TraceQL/Service Graph，选择 **TraceQL**，在输入框直接粘贴刚才真实的32位Trace ID后运行查询。不要在Service Name字段中填写ID。
- 也可选择Search，Service Name为`dms-client`，Span Name选择`dms.sdk_kv.set`或`dms.sdk_kv.get`，时间范围选最近15分钟，运行后点击表格中的ID。
- 旧Trace和其它应用请求仍保存在Tempo，因此结果不一定只有两行。用实际ID定位这次操作最直接。

SET根名应是`dms.sdk_kv.set`，GET是`dms.sdk_kv.get`。展开后看`dms.client.get`、Node处理、Meta解析、payload下载等子Span。不同Service的横条属于同一Trace，父子嵌套不能简单相加计算总耗时。薄 SDK 的普通 GET 仍访问 Node；Node 布局/Block 命中或SHM路径可能不经过全部远端步骤，这本身不是链路断裂。

## 4. 不依赖界面，用 API 验证

将GET命令刚输出的ID赋给变量；下面的提示文字必须换成真实ID：

```bash
export GET_TRACE_ID=在这里粘贴真实的32位ID
curl -fsS "http://127.0.0.1:3200/api/traces/$GET_TRACE_ID"
```

变量只是方便拼URL，**不是指定下一次请求的Trace ID**。复制一个历史示例ID不会查到本次数据。Span关闭后进入异步导出批次，不是整条链最后一端负责汇总；短命令退出会flush，服务端还可能等批次。404时先等数秒再查，并看Alloy/Tempo状态，不要立刻重复SET掩盖问题。

## 5. 日志和 Metrics 链接

Trace树中的日志链接跳到Loki。查询不到先在`DMS Loki`执行 `{job="dms"}`，确认Alloy读取了 `.local/manual/log`；再按 `trace_id` 过滤。没有debug请求日志时不保证每个Span都有关联日志。

Prometheus的Histogram Exemplar可带采样Trace链接，但Trace ID不是普通Metrics label。没有Exemplar不等于请求没有执行。

本手工端口不同于自动脚本默认端口；直接运行 `verify_tracing.sh` 可能查到另一组部署。使用本页输出ID/明确服务名核对。完毕后恢复默认Trace关闭和info级别，避免长期100%采样。
