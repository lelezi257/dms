# 白盒性能基线与验收合同

这份文档回答三个问题：一次基础操作理论上必须经过哪些步骤，当前实现离组成下界还有多远，以及后续改动怎样被同一把尺子验收。它不是产品排名，也不把不同部署形态的数字硬拼在一起。

机器可读事实维护在：

- [`benchmarks/whitebox/contract.json`](../../benchmarks/whitebox/contract.json)：Case、路径分类、前台 RPC、整段 payload copy 和 allocation 最小值。
- [`benchmarks/whitebox/baselines/lima-aarch64-2026-09-12.json`](../../benchmarks/whitebox/baselines/lima-aarch64-2026-09-12.json)：带环境指纹的首份 p50 基线。
- [`scripts/performance/evaluate_whitebox.py`](../../scripts/performance/evaluate_whitebox.py)：同环境候选结果的机器判定入口。

## 1. 首批固定路径

| 路径 | 前台最小调用图 | 为什么不能再少 |
| --- | --- | --- |
| 本地 SHM 热读 | `Client → local Node:Get` | 薄 SDK 只持有 key；Node 必须确认 Current 版本并保护本地 Block。返回普通 bytes 时仍有一次 SHM 到用户 buffer 的复制。 |
| Peer 首读 | `Client → reader Node:Get → Meta:ResolveObject → owner Node:PullBlock` | reader Node 首次既没有权威位置，也没有 bytes。副本登记已移出前台，但解析和直拉不能省。 |
| Peer 后热读 | `Client → reader Node:Get` | 首读已经在 reader Node 建立可复用副本和 Current cache，之后不再访问 Meta 或 owner。 |
| 4 KiB 本地写 | `SetInline → Meta:CommitVersion` | payload 可随控制请求携带；Meta 仍负责发布全局 Current。 |
| 1 MiB 本地 SHM 写 | `AllocateStaging → Set → Meta:CommitVersion` | Client 先取得 Node 管理的 Slot，直接写共享页，再由 Set/commit 宣布写完和发布版本；没有 Upload RPC。 |

这里的“RPC”统计前台 service 调用；不把 heartbeat、watch、连接预热和后台副本登记混入单次业务请求。copy/allocation 只统计整段对象 payload，不统计小 protobuf 结构、内核网络缓冲和 FUSE 内部缓冲。

## 2. 为什么 Peer 首读不直接和 Redis 判输赢

Redis 基线是中心节点已经同时持有 key 索引和数据的一次访问。DMS Peer 首读要先从独立 Meta 解析 owner，再从 owner 直拉到 reader Node，以换取后续访问的一次本地 Node 调用。因此它属于“架构首访成本”，但这不是无限预算：

- 实测 p50 必须不超过组成下界的 `1.25x`。
- 必须给出随后 Peer 热读的实测值，证明这次首访换来了本地复用。
- 不能把多余 RPC、重复摘要、同步副本登记或超过 `10%` 的未解释耗时藏进“架构成本”。

当前同环境结果：4 KiB Peer 首读为 `440.75 µs`，组成下界为 `409.31 µs`；1 MiB 为 `2.507 ms`，组成下界为 `2.355 ms`。两者分别是下界的 `1.08x` 和 `1.06x`。随后热读分别为 `90.61 µs` 和 `129.62 µs`。

## 3. 当前基线回答了什么

| Case | 4 KiB p50 | 1 MiB p50 | 结论 |
| --- | ---: | ---: | --- |
| SDK 本地热 GET | 92.80 µs | 121.76 µs | 4 KiB 与同机 Redis 持平；1 MiB 明显受益于本地 SHM。 |
| SDK 本地 SET | 245.47 µs | 696.50 µs | 小对象承担独立 Meta 发布成本；大对象仍优于同机 Redis 基线。 |
| SDK Peer 首读 | 440.75 µs | 2.507 ms | 比中心式基线慢，但已接近该部署下的组成下界。 |
| SDK Peer 后热读 | 90.61 µs | 129.62 µs | 首次拉取后回到本地热路径。 |
| 文件层 create | 2.761 ms | 5.561 ms | 相同文件层流程下优于 MinIO 对照。 |
| 文件层 read | 1.750 ms | 2.350 ms | Adapter 没有额外 Stat/Get；1 MiB 是两个并行范围读。 |
| 文件层 overwrite 4 KiB | 1.832 ms | 1.866 ms | 两列代表不同文件总大小，DMS 都只提交一个新的 4 KiB Slice。 |

这些数字只对 `lima-aarch64-3vm-2026-09-12` 环境形成硬基线。Redis、MinIO 和 DMS 的可靠性、协议及职责不同；对照只检查当前部署是否兑现既定架构收益。

## 4. 怎样生成候选结果

在同一组三台 VM、同一传输、同一挂载参数和同一采样口径中重新测量。候选 JSON 复用基线 schema，并满足：

1. `profile.id` 与基线完全一致；否则只能做趋势比较。
2. 每个 Case 至少 30 个正确性已验证的样本。
3. 填写实测 `p50_ns`、独立组成的 `lower_bound_p50_ns` 和同环境 `comparator_p50_ns`。
4. 按合同填写前台 RPC、整段 payload copy 与 allocation；实现若新增一步，不能顺手修改合同掩盖回归。
5. 对架构首访 Case 说明必要阶段，并附随后热读 p50。

运行：

```bash
python3 scripts/performance/evaluate_whitebox.py /path/to/candidate.json \
  --output /tmp/dms-whitebox-evaluation.json
```

不传候选文件时，脚本用已提交基线自检合同。它检查环境、Case 完整性、正确性、样本数、p50 回归、调用图账本、路径目标和未解释耗时；任一硬条件失败都会返回非零。

## 5. 修改合同和基线的规则

- 优化实现时只更新候选结果，不改合同。
- 用户语义、公开接口或一致性模型确实改变时，先通过设计审阅说明为何最小调用图改变，再单独修改合同。
- 只有同环境重复测量、正确性门禁通过且结果稳定时才能更新基线；Issue/PR 必须同时给出旧值、新值和根因。
- 跨机器、跨内核、跨虚拟化或跨 transport 的结果建立新 `profile.id` 和新基线文件，不覆盖旧环境。
- p99、并发、512 MiB/1 GiB、物理多机、RDMA/UB 尚未进入当前硬合同；后续应新增独立 Case，不从本轮 p50 外推。
