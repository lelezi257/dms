# 三 VM：部署进程并汇总 Metrics

目标是手工体验跨 Node 读和集群视角，不是 Meta HA。前提：已有三台相同 CPU 架构的 Linux VM，彼此 IP 可达；本项目的 `vm.sh` 只创建 `dms-dev`，不会创建这三台机器。

## 1. 部署布局

| VM | DMS进程 | 观测 |
| --- | --- | --- |
| n1 | Meta、Node、metrics host | Prometheus/Grafana/Loki/Tempo/Alloy |
| n2 | Node | 可选本机日志 Alloy |
| n3 | Node | 可选本机日志 Alloy |

三台 Node 使用19000 status /19200 Worker，Meta只在n1使用19100 status /19300 RPC，metrics host只在n1使用19400。示例 IP 为 `192.168.104.4/.5/.6`，**必须替换成实际 IP**；在各VM用 `ip -4 -brief address show scope global` 查询。开放相应私网端口，不暴露公网。

## 2. 在 Linux 构建一次，分发本地实验包

在开发 VM 源码根目录：

```bash
source scripts/env.sh
export CARGO_INCREMENTAL=0
./scripts/build.sh
./scripts/package.sh
```

输出是 `artifacts/dms-linux-aarch64.tar.gz`（x86_64 构建对应后缀为x86_64）。这是**本地实验组件包**，不是已发布0.1制品或SDK。用自己的文件传输工具复制到每台运行VM；同架构且系统运行依赖兼容，不跨架构使用。

每台 VM 中解压到新的实验目录，避免覆盖已有部署：

```bash
mkdir -p ~/dms-manual
tar -xzf /tmp/dms-linux-aarch64.tar.gz -C ~/dms-manual --strip-components=1
cd ~/dms-manual
sha256sum -c SHA256SUMS
cp config/dms.env.example config/dms.env
```

以下命令都在运行 VM 的 `~/dms-manual` 内执行，不再需要 Cargo/env.sh。

## 3. 每台编辑自己的 config/dms.env

| 字段 | n1 | n2 | n3 |
| --- | --- | --- | --- |
| `DMS_NODE_ID` | node-n1 | node-n2 | node-n3 |
| `DMS_NODE_IP` | 192.168.104.4 | 192.168.104.5 | 192.168.104.6 |
| `DMS_META_ENDPOINT` | http://192.168.104.4:19300 | 同左 | 同左 |
| `DMS_CLIENT_ENDPOINT` | http://192.168.104.4:19200 | http://192.168.104.5:19200 | http://192.168.104.6:19200 |

其它字段保留样例默认。`DMS_NODE_IP` 必须是其它Node可访问的地址，不能填0.0.0.0或127.0.0.1，否则注册的位置不能供远端读取。配置文件是shell变量，`cluster.sh` 将其转成CLI，并非服务原生TOML。

## 4. 先 Meta，再 Node，再应用

n1：

```bash
./scripts/cluster.sh meta start
./scripts/cluster.sh meta status
```

等 Meta status 成功，在三台分别执行：

```bash
./scripts/cluster.sh node start
./scripts/cluster.sh node status
```

只在 n1：

```bash
./scripts/cluster.sh client start
./scripts/cluster.sh client status
```

`start` 本身是后台启动，不代表READY；若首次status失败，检查 `log/*.fallback.log` 与进程PID，不要把旧listener当新进程。

## 5. 跨节点读

n1：

```bash
DMS_ENDPOINT=http://192.168.104.4:19200 ./bin/sdk-kv set tutorial/peer from-n1
```

n2：

```bash
DMS_ENDPOINT=http://192.168.104.5:19200 ./bin/sdk-kv get tutorial/peer from-n1
```

n3替换为 `.6` 执行相同GET。预期 `get ok`。数据在n1写入，n2/n3从Meta查位置并从Peer拉取；这不证明同步多副本 durability。只保留单份内存的写在原节点故障时仍有丢失风险。

## 6. n1 汇总指标

按[观测说明](observability.md)在n1安装Docker，然后：

```bash
./scripts/metrics-targets.sh three 192.168.104.4 192.168.104.5 192.168.104.6
./scripts/observability.sh up
./scripts/observability.sh status
```

浏览器访问n1 `:9090/targets`，应有3个Node+1个Meta+1个Client宿主，共5个UP。Grafana访问n1 `:3000`。按节点查询：

```promql
sum by (instance) (dms_node_replica_bytes_total{direction="receive"})
```

本页使用实验包的默认端口，因此可额外执行：

```bash
./scripts/verify_metrics.sh --topology three-node 192.168.104.4 192.168.104.5 192.168.104.6
```

指标数和名字以当前源码合同为准，不以历史某次运行的固定数字作为产品能力证明。

## 7. 停止与边界

在各实验目录用 `./scripts/cluster.sh all stop` 停本实验；n1用 `./scripts/observability.sh down` 停观测。不要删 journal 或观测卷作为普通停止步骤。该运行脚本依赖本目录PID文件；不要复用其它部署的run目录。

本次文档更新不等于重新完成三台VM验证。正式制品和生产环境验收另行完成；Node内存恢复、长期GC和Meta HA不由三节点拓扑自动获得。
