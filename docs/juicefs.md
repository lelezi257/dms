# JuiceFS 接入 DMS

本阶段把 DMS 接到 JuiceFS 的**对象存储后端**，不是重新实现文件系统。文件路径、inode、目录、文件长度和文件块映射仍由 JuiceFS 管理；DMS 保存后端 key 对应的 bytes、版本和节点位置。文件系统元数据 Redis 与 DMS Meta 是两套不同职责，不能省掉其中一个。

这是候选接入指南，不代表故障验收或三 VM 性能验收已通过。不要存放唯一的重要数据：当前采用易失内存策略，即使文件 `fsync` 成功，也不承诺 Node 丢失内存或机器掉电后的数据恢复。

## 1. 接入边界与本阶段配置

```text
文件写入 /data/example.bin
  → JuiceFS 文件系统处理：文件偏移 → 文件块 → 后端对象 key
  → DMS 对象适配器：Put/Get/Delete/Head/List
  → 纯 Go SDK → 本机或远端 DMS Node → DMS Meta

文件路径 / inode / 块映射 → JuiceFS 元数据 Redis
对象 key / 版本 / Block 位置 → DMS Meta
对象 bytes → DMS Node 内存
```

适配器位于含 DMS 后端的 JuiceFS 源码 `pkg/object/dms.go`，依赖正式 Go SDK 候选包；不把 DMS 类型扩散到 FUSE、inode 或文件块格式中。DMS 源码仓不包含该 JuiceFS 源码树，普通 JuiceFS 二进制也不会自动获得 `--storage dms` 能力。

| JuiceFS 对象操作 | Go SDK 调用 | 关键语义 |
| --- | --- | --- |
| Put | Set | 先读完本次对象再提交；Reader 失败不能提交半个对象 |
| 完整 Get | Get | 缺失转换为对象不存在，网络错误仍是错误 |
| 范围 Get | Stat + GetWithOptions | 先确定长度/版本，再读该固定版本范围，避免混用两版数据 |
| Delete | Del | 对已缺失对象重复删除也成功 |
| Head | Stat | 返回实际长度、稳定修改时间，不下载对象 |
| List / ListAll | Scan 分页 | 使用不透明游标；失败不伪装成正常分页结束 |

阶段限制：

- 默认文件块大小 4MiB。适配器单次后端 Put 上限 **8MiB**，不是文件大小上限；512MiB/1GiB 文件由多个对象组成。
- Put 在适配器内暂存对象，默认最多 4 个并发 Put；`DMS_JUICEFS_MAX_INFLIGHT_PUTS` 可设 1～64，不代表整个进程 RSS 上限。
- 本阶段挂载显式使用 `--backup-meta 0`，暂不把可能较大的文件系统元数据备份对象写入 DMS。它不是关闭正常后台任务；**不要加 `--no-bgjob`**，删除和其它正常后台流程仍需运行。
- 不声明 delimiter 列举、multipart 或其它未实现对象能力。TLS/可靠性边界与 [Go SDK](go-sdk.md) 相同。
- 以下演示禁用 JuiceFS 本地数据缓存，便于观察 DMS 路径；Node 缓存仍然存在。重挂载只清理文件系统客户端状态，不等于清掉 Node 缓存。

## 2. 先准备环境

源码接入基线在 [DMS分支](https://github.com/lelezi257/dms/tree/integration/juicefs-baseline) 与 [JuiceFS fork](https://github.com/lelezi257/juicefs-dms)。配套固定身份如下；没有正式Release下载包。

| 组成 | 固定身份 |
| --- | --- |
| DMS服务与SDK代码基线 | `f4555eac23190ceef555b284366e4623c48fb72b` |
| Go SDK | `v0.0.0-20260910013452-f4555eac2319` |
| JuiceFS接入 | `42539ab68e3340baf02c817b5c08e2eb63095b1b` |
| JuiceFS原版基线 | `0b90c7db5a929ae6adc5faad948d108efd2c99f9`，v1.4.1 |

DMS后续仅文档修正不会改变上述服务代码基线；复现实验仍记录实际检出的完整提交，不以分支名代替固定身份。

所有命令在 Linux VM 内执行。先按[单 VM 手册](local-single-vm-manual.md)准备编译环境，还需 FUSE3（含 `/dev/fuse`、`fusermount3`）和 `redis-server`。

在 DMS 源码根执行；如果已有匹配的组件制品，可把 BIN 指向其解压目录，不重复编译：

```bash
source scripts/env.sh
scripts/build.sh
export REPO="$PWD"
export BIN="$CARGO_TARGET_DIR/release"
export RUN="$REPO/.local/juicefs-demo"
export JFS_BIN="$REPO/.local/bin/juicefs"
mkdir -p "$RUN/redis" "$RUN/mnt-a" "$RUN/mnt-b"
```

从公开fork构建**含DMS adapter**的二进制，不使用上游Release替代。以下在Linux另一个终端、DMS源码的同级目录执行，构建需要Go 1.25或兼容工具链和C编译器：

```bash
git clone https://github.com/lelezi257/juicefs-dms.git
cd juicefs-dms
git checkout --detach 42539ab68e3340baf02c817b5c08e2eb63095b1b
GOPROXY=https://proxy.golang.org,direct go build -mod=readonly -o juicefs .
```

将本次生成的 `juicefs` 复制到前面设置的 `$JFS_BIN`，并记录 `sha256sum`；Go依赖由go.mod固定并从远端下载，不使用本地replace或候选proxy。每个新终端都先设置上述变量，再运行下面相应命令；不要在一个后台长命令中隐藏全部服务。

使用新的运行目录和空 Redis 数据库；如果端口或数据库已有用户服务/数据，换一套隔离地址，不停止原服务、不执行 FLUSHDB。

## 3. 单 VM：一个 Node，两个挂载

服务分别在独立终端前台运行，方便直接观察输出。

**终端 1：DMS Meta。** 不配置 Journal 的本示例只用于易失环境。

```bash
"$BIN/dms-meta" serve --node-id demo-meta \
  --grpc-address 127.0.0.1:25300 \
  --health-address 127.0.0.1:25100 \
  --log-path "$RUN/meta.log"
```

**终端 2：DMS Node。** 2GiB 内存预算，64MiB Region，保留其它正常默认配置。

```bash
"$BIN/dms-node" serve --node-id demo-node \
  --worker-tcp-address 127.0.0.1:25200 \
  --worker-uds-path "$RUN/worker.sock" \
  --meta-endpoint http://127.0.0.1:25300 \
  --health-address 127.0.0.1:25000 \
  --arena-capacity-bytes 2147483648 --region-size-bytes 67108864 \
  --log-path "$RUN/node.log"
```

**终端 3：文件系统元数据 Redis。** 此演示 Redis 同样不持久化，不是生产部署配置。

```bash
redis-server --bind 127.0.0.1 --port 26379 \
  --save "" --appendonly no --dir "$RUN/redis"
```

**准备挂载终端：先选择连接方式。** `lab` 是卷内保存的稳定别名，每个挂载进程把它映射到自己的 Node。

```bash
export META_URL=redis://127.0.0.1:26379/0
export DMS_JUICEFS_ENDPOINTS="{\"lab\":\"unix://$RUN/worker.sock\"}"
export DMS_SHARED_MEMORY=true
```

测试 TCP 时替换为下列两项，其余文件系统操作相同：

```bash
export DMS_JUICEFS_ENDPOINTS='{"lab":"http://127.0.0.1:25200"}'
export DMS_SHARED_MEMORY=false
```

每个新卷只 format 一次；`--block-size 4096` 的单位是 KiB，即 4MiB。`--trash-days 0` 是本演示不保留回收站的明确选择，不缩短 DMS 的读保护或后台回收安全条件。

```bash
"$JFS_BIN" --no-agent format --storage dms --bucket dms://lab \
  --block-size 4096 --trash-days 0 "$META_URL" dms-demo
```

两个挂载终端分别选择 MOUNT 和 CACHE，再执行同一个前台 mount 命令；两者必须使用相同 META_URL 和连接环境。

```bash
# 挂载终端 A；终端 B 改成 mnt-b 和 cache-b。
export MOUNT="$RUN/mnt-a"
export CACHE="$RUN/cache-a"
"$JFS_BIN" --no-agent mount --no-usage-report \
  --backup-meta 0 --cache-size 0 --cache-dir "$CACHE" \
  --attr-cache 0 --entry-cache 0 "$META_URL" "$MOUNT"
```

**操作终端：写后第一次读就校验。** 5MiB 会跨越默认 4MiB 文件块边界。

```bash
dd if=/dev/urandom of="$RUN/mnt-a/example.bin" \
  bs=1M count=5 conv=fsync status=none
sha256sum "$RUN/mnt-a/example.bin" "$RUN/mnt-b/example.bin"
cmp "$RUN/mnt-a/example.bin" "$RUN/mnt-b/example.bin"
```

两个摘要应相同，cmp 应成功。失败时先保留日志，不通过重复写或延迟读把失败遮掉。本例只证明单 Node 的真实文件系统路径，不能证明跨 Node 首读或重读命中。

## 4. 三 VM：写在 A，首次读在 B

| VM | 进程 | 地址示意 |
| --- | --- | --- |
| A | Node A、挂载 A | Node 私网 IP:25200；本机 worker.sock |
| B | Node B、挂载 B | Node 私网 IP:25200；本机 worker.sock |
| C | DMS Meta、文件系统元数据 Redis | Meta 私网 IP:25300；Redis 私网 IP:26379 |

三台分别准备第 2 节的环境、运行目录和匹配的二进制。变量中的占位符换为实际私网 IP；端口仅开放给隔离实验网络，不向公网暴露明文服务。

**C：** `META_IP` 为 C 私网 IP，按下面启动；两个命令放不同终端。

```bash
export META_IP='<C 私网 IP>'
"$BIN/dms-meta" serve --node-id demo-meta \
  --grpc-address "$META_IP:25300" \
  --health-address 127.0.0.1:25100 --log-path "$RUN/meta.log"
```

```bash
redis-server --bind "$META_IP" --port 26379 \
  --save "" --appendonly no --dir "$RUN/redis"
```

**A/B：** 各自设置 NODE_ID 和 NODE_IP；Node 的 TCP 地址必须被对端 VM 访问，不能填 loopback，否则跨 Node 拉取无法到达。

```bash
export NODE_ID=demo-a  # B 使用 demo-b
export NODE_IP='<当前 VM 私网 IP>'
export META_IP='<C 私网 IP>'
"$BIN/dms-node" serve --node-id "$NODE_ID" \
  --worker-tcp-address "$NODE_IP:25200" \
  --worker-uds-path "$RUN/worker.sock" \
  --meta-endpoint "http://$META_IP:25300" \
  --health-address 127.0.0.1:25000 \
  --arena-capacity-bytes 2147483648 --region-size-bytes 67108864 \
  --log-path "$RUN/node.log"
```

**A/B 挂载终端：** 两边卷别名和文件系统元数据地址相同，但 `lab` 各自连接本机 Node。路径相同只是各自 VM 的本地路径，不是跨 VM 共享 socket。

```bash
export META_URL="redis://$META_IP:26379/0"
export DMS_JUICEFS_ENDPOINTS="{\"lab\":\"unix://$RUN/worker.sock\"}"
export DMS_SHARED_MEMORY=true
```

只在 A 按第 3 节 format 一次；A 挂载 mnt-a，B 挂载 mnt-b，使用同一 mount 命令。A 执行上面的 dd 和本地 sha256sum；B 对 `"$RUN/mnt-b/example.bin"` 执行 sha256sum，比较摘要。

路径应为：A 文件写 → Node A 保存 bytes → Meta C 登记位置；B 首读 → Node B 查位置并从 Node A 拉取 → B 返回文件内容。正常再次读可能命中 Node B，不应要求每次都拉取 A。

进一步验证时分别记录：首次读取、B 重挂载后的再次读取、覆盖后的新版本读取。重挂载清掉 JuiceFS 客户端状态，保留 Node B 缓存；Node metrics 的 Peer PullBlock 计数可以帮助区分是否再次跨节点传输。实际 RPC 次数、回收效果和性能结论必须由对应验收记录证明，本页不预填“通过”。

## 5. 删除、回收与退出

删除文件只表示命名空间变化。文件仍被打开时，其数据仍可能有效；旧版本、共享读保护和后台工作未结束时，bytes 也不能立即复用。正常 GC 应在对象确实不再被引用后回收，**不能因内存压力直接丢掉 live 对象的唯一副本**。

本阶段 `--trash-days 0` 不等于同步物理释放。批量验证需观察正常删除与回收完成再重复，不用重启 Node、删除服务数据或缩短保护窗口假造回收通过。Node `/metrics`、组件日志和文件 SHA 分别证明状态、失败原因与内容，探活不能替代内容校验。

```bash
# 单 VM 在本机执行两条；三 VM 分别卸载各自挂载。
fusermount3 -u "$RUN/mnt-a"
fusermount3 -u "$RUN/mnt-b"
```

先卸载，再在自己启动服务的终端按 Ctrl-C 停 Node、Meta、Redis。不要按进程名批量 kill，也不要自动清理已有运行目录。诊断保留 `$RUN` 中本次日志；敏感 key/value 不应写入公开报告。

## 6. 怎样观察本次文件操作

先看实际服务地址，不必立即部署完整观测栈。在 Node 所在 VM、服务仍运行时执行：

```bash
curl -fsS http://127.0.0.1:25000/metrics \
  | grep -E '^dms_(rpc_server_requests_total|node_replica_bytes_total|node_arena_(allocated|logical|quarantined)_bytes)'
```

在读文件前后各取一次，比较**增量**：A 的 `PeerService/PullBlock` 与发送 bytes 可以证明 B 是否实际拉取；B 重挂载再次读时，这两项不再增长才是 Node 数据复用的证据。范围读仍可能有 Stat、Get 和 Meta 请求；SHM 读还可能有首次 Region 映射与读保护归还，不能用“没有 Peer”推导“没有 RPC”。删除后 allocated/logical/quarantined 分别表示物理分配、逻辑活数据和仍隔离的容量，不要求进程 RSS 立即归零。

组件失败先读本轮 `"$RUN/node.log"`、`"$RUN/meta.log"` 和前台挂载输出，保留数字错误码及上下文。需要集中查看时，沿用[观测总览](observability.md)、[三节点 Metrics 手册](metrics-three-node-manual.md)与[三节点 Trace 手册](tracing-three-node-manual.md)，将 targets、日志目录和 OTLP 地址替换为本轮实例，不复用或覆盖其它实验的配置。

当前 Go SDK 尚未接入跨语言 Trace 上下文与宿主观测注入；服务端已有 Span 不等于自动形成完整的“文件调用→Go SDK→Node→Meta”同一 Trace。本阶段用文件操作计时、JuiceFS 指标、Node/Meta 指标与日志交叉定位，不把 Rust SDK 的 Trace 示例冒充 Go 接入证据。

## 7. 当前还不能由本指南推导的结论

- fsync 成功不等于内存数据具备持久可靠性；Meta Journal 也不会替 Node 保存 value。
- 单 VM 示例不替代跨 Node、故障、正常 GC 和内存预算验收。
- 三 VM 的基础手动步骤不等于完整样本性能测试，更不能据此声称全部文件系统语义或生产可靠性已通过。

发布前由候选验收记录明确标注功能、故障/回收、性能各自的结果与未测项。
