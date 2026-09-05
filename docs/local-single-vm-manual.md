# 单 VM：手工启动，再执行 SET / GET

这份教程启动真实的 Client → Node → Meta。SDK 是库，`sdk_kv` 才是使用 SDK 的应用进程。所有运行在 Linux VM 内完成，不需要 Docker；只有后面的观测栈需要 Docker。

## 1. 进入环境并编译

Mac 在源码根目录执行：

```bash
./scripts/vm.sh up
./scripts/vm.sh shell
```

VM 内：

```bash
source scripts/env.sh
export CARGO_INCREMENTAL=0
./scripts/build.sh
mkdir -p .local/manual/run .local/manual/journal .local/manual/log
```

教程使用独立端口：Node gRPC `25200`、Node status `25000`、Meta gRPC `25300`、Meta status `25100`。如已占用，请整体替换；不要停止不属于本教程的进程。

## 2. 终端 A：先启动 Meta

在源码根目录前台运行，保留此终端：

```bash
"$CARGO_TARGET_DIR/release/dms-meta" serve \
  --node-id manual-meta \
  --health-address 0.0.0.0:25100 \
  --grpc-address 127.0.0.1:25300 \
  --journal-dir "$PWD/.local/manual/journal" \
  --log-path "$PWD/.local/manual/log/dms-meta.log"
```

Meta 保存 key 当前版本、Block 位置和提交结果；`journal-dir` 保存其 WAL/snapshot，不保存 value 本体。

## 3. 终端 B：再启动 Node

另外打开 VM shell，在源码根目录加载环境，然后运行：

```bash
source scripts/env.sh
"$CARGO_TARGET_DIR/release/dms-node" serve \
  --node-id manual-node \
  --health-address 0.0.0.0:25000 \
  --worker-tcp-address 127.0.0.1:25200 \
  --worker-uds-path "$PWD/.local/manual/run/worker.sock" \
  --meta-endpoint http://127.0.0.1:25300 \
  --log-path "$PWD/.local/manual/log/dms-node.log"
```

Node 先注册 Meta，再提供业务服务。`worker.sock` 是本地 gRPC 控制通道；旁边的 `worker.fd.sock` 传递共享内存 Region 的 FD。TCP 和 UDS 都连接同一个 Node，不是两份数据。

## 4. 终端 C：检查并读写

同样进入 VM 源码根目录：

```bash
source scripts/env.sh
curl -fsS http://127.0.0.1:25100/readyz
curl -fsS http://127.0.0.1:25000/readyz
export DMS_ENDPOINT=http://127.0.0.1:25200
"$CARGO_TARGET_DIR/release/examples/sdk_kv" set tutorial/hello world
"$CARGO_TARGET_DIR/release/examples/sdk_kv" get tutorial/hello world
"$CARGO_TARGET_DIR/release/examples/sdk_kv" del tutorial/hello
```

预期：ready 请求成功；写入打印 `set ok` 和 version；读取打印 `get ok`；删除打印 `deleted=true`。GET 示例的第三个参数是**期望值**，会做字节比较，不是普通字符串打印命令。SET 成功后不需要 sleep 再 GET。

每次执行 `sdk_kv` 都是新 Client 进程，不能用它证明同一 SDK 实例的缓存命中。验证同一进程的双 Client、range 和 batch：

```bash
"$CARGO_TARGET_DIR/release/examples/sdk_api"
```

预期最后输出 `set/get/del + range + batch APIs passed`。缓存断流/过期不能继续把旧值当 Current；其它 writer 成功提交可能需要等待缓存失效 ACK 或旧租约到期。

## 5. 只切换通道，再验证本地 SHM

```bash
export DMS_ENDPOINT="unix://$PWD/.local/manual/run/worker.sock"
export DMS_SHARED_MEMORY=true
"$CARGO_TARGET_DIR/release/examples/sdk_kv" set tutorial/shm shared-value
"$CARGO_TARGET_DIR/release/examples/sdk_kv" get tutorial/shm shared-value
"$CARGO_TARGET_DIR/release/examples/sdk_shared_memory"
```

普通 SET/GET 仍是相同用户接口。显式 `sdk_shared_memory` 示例另外演示 `allocate_write → commit_shared → get_view`；不是只有这个 API 才能使用 SHM。Region 首次映射需 FD Broker，后续分配通常复用映射与 offset/length。

当前已导出的 SHM allocation 在释放后采取隔离而非不安全复用，会继续占用容量；完整 ViewEpoch 驱动的回收尚未完成。不要用长时间反复覆盖压测把它当成无限容量缓存。

## 6. 日志、停止与重启

```bash
tail -f .local/manual/log/dms-node.log
```

停止本教程进程：在其前台终端先对 Node 按 Ctrl-C，再对 Meta 按 Ctrl-C。再次运行原命令会复用 Meta journal；Node 内存 value 不保证恢复。不要删除 journal 来处理普通连接问题。

`scripts/deploy.sh` 是可重置的开发实验脚本：会停止其管理的进程、删除指定 Meta journal，并截断日志，而且启动 metrics host。**不要把它当成保留数据的普通部署或重启命令。** 本教程没有使用它。

下一步：[Metrics 界面](metrics-single-node-manual.md)、[SET/GET Trace 界面](tracing-single-node-manual.md)、[配置](configuration.md)。
