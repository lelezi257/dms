# DMS Native Filesystem 与 MooseFS 性能摸底

> Preview 判定：**NOT_READY**。本报告区分公平内存介质 lane 与真实虚拟磁盘 lane，后者不用于宣称同等可靠性。

## P1 稳定热路径补充（2026-09-17）

原始摸底中 `workspace.local_hot` 和 `workspace.peer_repeat` 的主要差距已经收口。新实现不再为普通读逐文件查询 access ACL，也不会为没有持锁的 FUSE owner 调用 Meta release；FUSE 正缓存只使用 Meta grant 的剩余租约，远端 namespace 变更仍在 Watch ACK 前精确撤销 inode/dentry。

“稳定热路径”在采集前让 DMS 与 MooseFS 对同一工作集各 warmup 一次；warmup 结果独立保存，不进入正式样本或 before/after RPC 差值。这只排除每目录一次的冷授权恢复，不掩盖持续 RPC。

| 独立运行 | Case | DMS p50 | MooseFS p50 | p50 比值 | DMS p95 | MooseFS p95 | p95 比值 | 前台 Meta/Peer RPC |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| run-1 | `workspace.local_hot` | 154.33 µs | 208.25 µs | 0.741 | 233.58 µs | 359.29 µs | 0.650 | 0 / 0 |
| run-1 | `workspace.peer_repeat` | 152.17 µs | 212.54 µs | 0.716 | 187.00 µs | 393.09 µs | 0.476 | 0 / 0 |
| run-2 | `workspace.local_hot` | 149.25 µs | 206.17 µs | 0.724 | 223.42 µs | 382.09 µs | 0.585 | 0 / 0 |
| run-2 | `workspace.peer_repeat` | 152.75 µs | 212.62 µs | 0.718 | 185.67 µs | 372.36 µs | 0.499 | 0 / 0 |

P1 evaluator 为 `PASS`；机器摘要见 [`native-fs-hot-path-lima-aarch64-2026-09-17.json`](../../benchmarks/whitebox/baselines/native-fs-hot-path-lima-aarch64-2026-09-17.json)。本报告后续章节保留 P0 原始摸底，作为优化前证据；整体 Preview 仍需继续完成 namespace/mutation、write-through 和 peer-first 阶段。

## 1. 对比拓扑与原则

- A：写入端，也是 DMS Block owner / MooseFS 唯一 ChunkServer。
- B：远端首次读取与复读端。
- C：DMS Meta / MooseFS Master。
- memory lane：两端 payload 都驻内存；至少 5 轮、后端顺序交替。
- disk lane：MooseFS 使用 VM 虚拟磁盘，DMS 仍为内存对象系统，只展示产品部署差异。
- 两端执行完全相同的 POSIX workload；MooseFS 使用 goal=1，避免复制数干扰。
- DMS source SHA：`b36f570c85e7230f7cabf97afd0768fb85d75285`；实际二进制 SHA-256 记录在结果 JSON。

## 2. 公平 memory lane

| Case | DMS p50 | MooseFS p50 | DMS p95 | MooseFS p95 | DMS 吞吐 MiB/s | MooseFS 吞吐 MiB/s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `workspace.create` | 2966.19 µs | 832.08 µs | 4853.62 µs | 1285.45 µs | 3.68 | 2.24 |
| `workspace.local_hot` | 1150.20 µs | 342.75 µs | 1531.74 µs | 500.62 µs | 15.23 | 49.35 |
| `workspace.stat` | 760.08 µs | 166.37 µs | 972.70 µs | 316.00 µs | 0.00 | 0.00 |
| `workspace.peer_first` | 1956.32 µs | 443.82 µs | 2731.89 µs | 673.56 µs | 8.76 | 37.41 |
| `workspace.peer_repeat` | 1165.68 µs | 214.87 µs | 1425.05 µs | 417.99 µs | 15.27 | 73.11 |
| `workspace.patch` | 2368.90 µs | 661.33 µs | 3157.41 µs | 950.62 µs | 0.40 | 1.37 |
| `workspace.create_delete` | 4959.91 µs | 1073.74 µs | 5829.41 µs | 1493.83 µs | 0.19 | 0.85 |
| `sequential_512m.write` | 3318322.49 µs | 540593.64 µs | 3426950.23 µs | 651478.91 µs | 154.69 | 921.58 |
| `sequential_512m.local_read` | 241100.42 µs | 439582.70 µs | 244526.97 µs | 455208.22 µs | 1215.49 | 803.96 |
| `sequential_512m.peer_first` | 3946341.85 µs | 1438698.60 µs | 4038374.91 µs | 1511110.05 µs | 125.75 | 318.36 |
| `sequential_512m.peer_repeat` | 239706.38 µs | 236502.74 µs | 315426.89 µs | 252671.33 µs | 1162.29 | 1215.40 |

## 3. Preview 门槛

| Case | 指标 | 比值 | 门槛 | 结果 |
| --- | --- | ---: | ---: | --- |
| `workspace.local_hot` | DMS/MooseFS latency | 3.301 | 1.100 | FAIL |
| `workspace.peer_first` | DMS/MooseFS latency | 4.402 | 1.150 | FAIL |
| `workspace.peer_repeat` | DMS/MooseFS latency | 5.422 | 1.100 | FAIL |
| `sequential_512m.peer_first` | DMS/MooseFS throughput | 0.390 | 0.900 | FAIL |
| `sequential_512m.peer_repeat` | DMS/MooseFS throughput | 0.969 | 0.900 | PASS |

## 4. 架构固有成本

- **workspace.peer_first / sequential_512m.peer_first**：远端首次读取必须解析 Exact Version 并从拥有 Block 的 Node 拉取 payload；这是分布式近计算架构的冷访成本。
- **MooseFS peer_first**：MooseFS 同样需要从 Master 获取 chunk 位置并从 A 的 ChunkServer 读取；两端都不是纯本地 memcpy。

## 5. 实现审计点

- **workspace.local_hot**：800 次文件读取额外产生 GetFilesystemXattr=800、ReleaseFilesystemLockOwner=800、GetFilesystemInode=40 次 Meta RPC。既有热路径合同要求 0 Meta/Peer；这是实现回归，不是架构成本。
- **workspace.peer_first / workspace.peer_repeat**：首读按对象产生 PullBlock=800，属于缺失 Block 接管；但同时又有 GetXattr=800 和 ReleaseLockOwner=800。复读已无 PullBlock，却仍分别产生 GetXattr=800、ReleaseLockOwner=800；因此小文件复读差距来自控制路径，而不是 payload。
- **sequential_512m.write**：5 个 512 MiB 文件产生 CommitFilesystemVersion=2560，即每个 1 MiB write-through callback 都同步发布一个版本；这解释了写吞吐差距，后续应优化提交合并/流水线，而不是 payload memcpy。
- **sequential_512m.peer_first**：5 次首读产生 PullBlock=2560、ReportReplicas=2322。分块拉取是数据路径需要，但逐块 RPC 与副本上报放大不是不可避免的架构税。
- **sequential_512m.local_read / peer_repeat**：DMS 本地读吞吐 1215.49 MiB/s，高于 MooseFS 803.96 MiB/s；跨节点接管后的复读吞吐比值 0.969，已通过门槛。这证明本地 DataCore/payload 路径有效，优化重点应放在控制请求放大。

## 6. 白盒证据导读

### workspace.local_hot

- DMS copy path：Node 本地 Block→FUSE 用户缓冲区（完整字节复制阶段 1）。
- MooseFS copy path：client/page cache 或 ChunkServer→用户缓冲区（模型阶段 1）。
- 执行端 A 网络字节：DMS RX 315605 / TX 1241013；MooseFS RX 230822 / TX 192309。
- DMS 发起端 RPC（5 轮合计）：

| DMS method | 次数 | client observe 总耗时 |
| --- | ---: | ---: |
| `GetFilesystemInode` | 40 | 9.72 ms |
| `GetFilesystemXattr` | 800 | 171.61 ms |
| `ReleaseFilesystemLockOwner` | 800 | 184.39 ms |

- MooseFS client operation（5 轮合计）：

| MooseFS operation | 次数 |
| --- | ---: |
| `mfs_client:fsync` | 58 |
| `mfs_client:lookup` | 845 |
| `mfs_client:open` | 800 |
| `mfs_client:read` | 211 |
| `mfs_client:total` | 1970 |
| `mfs_client:write` | 56 |

### workspace.peer_first

- DMS copy path：Peer Block→接收 Arena→FUSE→用户缓冲区（完整字节复制阶段 3）。
- MooseFS copy path：ChunkServer→FUSE client→用户缓冲区（模型阶段 2）。
- 执行端 B 网络字节：DMS RX 17311231 / TX 2049751；MooseFS RX 16598014 / TX 377087。
- DMS 发起端 RPC（5 轮合计）：

| DMS method | 次数 | client observe 总耗时 |
| --- | ---: | ---: |
| `GetFilesystemInode` | 5 | 2.77 ms |
| `GetFilesystemXattr` | 800 | 173.82 ms |
| `LookupFilesystemEntry` | 845 | 218.17 ms |
| `PullBlock` | 800 | 268.19 ms |
| `ReleaseFilesystemLockOwner` | 800 | 193.07 ms |
| `ReportReplicas` | 255 | 72.90 ms |

- MooseFS client operation（5 轮合计）：

| MooseFS operation | 次数 |
| --- | ---: |
| `mfs_client:lookup` | 845 |
| `mfs_client:open` | 800 |
| `mfs_client:read` | 103 |
| `mfs_client:total` | 1748 |

### workspace.peer_repeat

- DMS copy path：接收 Node 本地 Block→FUSE 用户缓冲区（完整字节复制阶段 1）。
- MooseFS copy path：client/page cache→用户缓冲区（模型阶段 1）。
- 执行端 B 网络字节：DMS RX 308238 / TX 1246903；MooseFS RX 230608 / TX 230557。
- DMS 发起端 RPC（5 轮合计）：

| DMS method | 次数 | client observe 总耗时 |
| --- | ---: | ---: |
| `GetFilesystemXattr` | 800 | 175.83 ms |
| `Heartbeat` | 8 | 3.73 ms |
| `ReleaseFilesystemLockOwner` | 800 | 182.56 ms |
| `RenewFilesystemInodeReferences` | 8 | 2.46 ms |

- MooseFS client operation（5 轮合计）：

| MooseFS operation | 次数 |
| --- | ---: |
| `mfs_client:lookup` | 845 |
| `mfs_client:open` | 800 |
| `mfs_client:read` | 217 |
| `mfs_client:total` | 1862 |

### sequential_512m.peer_first

- DMS copy path：Peer Block→接收 Arena→FUSE→用户缓冲区（完整字节复制阶段 3）。
- MooseFS copy path：ChunkServer→FUSE client→用户缓冲区（模型阶段 2）。
- 执行端 B 网络字节：DMS RX 2810344574 / TX 32004030；MooseFS RX 2807962406 / TX 11793339。
- DMS 发起端 RPC（5 轮合计）：

| DMS method | 次数 | client observe 总耗时 |
| --- | ---: | ---: |
| `GetFilesystemInode` | 5 | 3.22 ms |
| `GetFilesystemXattr` | 5 | 1.78 ms |
| `Heartbeat` | 10 | 9.75 ms |
| `LookupFilesystemEntry` | 5 | 9.06 ms |
| `PullBlock` | 2560 | 10642.72 ms |
| `ReleaseFilesystemLockOwner` | 5 | 2.98 ms |
| `RenewFilesystemInodeReferences` | 10 | 3.08 ms |
| `ReportReplicas` | 2322 | 4635.30 ms |

- MooseFS client operation（5 轮合计）：

| MooseFS operation | 次数 |
| --- | ---: |
| `mfs_client:getattr` | 5 |
| `mfs_client:lookup` | 5 |
| `mfs_client:open` | 5 |
| `mfs_client:rchunk` | 35 |
| `mfs_client:read` | 19403 |
| `mfs_client:total` | 19453 |

## 7. disk lane（单独陈述）

本轮完成 1 轮。MooseFS payload/metadata 位于 VM 虚拟磁盘，DMS 仍是内存对象系统；该 lane 只回答真实部署差异，不宣称可靠性介质等价。

| Case | DMS p50 | MooseFS p50 | DMS 吞吐 MiB/s | MooseFS 吞吐 MiB/s |
| --- | ---: | ---: | ---: | ---: |
| `workspace.create` | 2837.44 µs | 833.58 µs | 3.40 | 2.59 |
| `workspace.local_hot` | 1127.33 µs | 352.83 µs | 15.95 | 45.75 |
| `workspace.stat` | 748.20 µs | 162.71 µs | 0.00 | 0.00 |
| `workspace.peer_first` | 1848.04 µs | 482.91 µs | 9.37 | 34.13 |
| `workspace.peer_repeat` | 1141.10 µs | 204.91 µs | 15.49 | 79.16 |
| `workspace.patch` | 2264.90 µs | 669.91 µs | 0.41 | 1.31 |
| `workspace.create_delete` | 4967.13 µs | 1000.24 µs | 0.19 | 0.90 |
| `sequential_512m.write` | 3215566.33 µs | 429593.78 µs | 159.22 | 1191.77 |
| `sequential_512m.local_read` | 257563.53 µs | 477394.41 µs | 1154.19 | 770.92 |
| `sequential_512m.peer_first` | 4218076.03 µs | 1531579.14 µs | 116.39 | 298.22 |
| `sequential_512m.peer_repeat` | 253234.06 µs | 256243.78 µs | 1183.03 | 1160.62 |

## 8. 结论与下一轮边界

1. **Preview 暂不放行**：5 项性能门槛仅 `sequential_512m.peer_repeat` 通过。
2. **架构优势已被证明**：512 MiB 本地热读明显领先；跨节点接管后的复读已基本持平。
3. **P0：修复小文件热路径回归**：本地热读和 peer repeat 删除逐文件 GetXattr / ReleaseLockOwner Meta RPC，并恢复既有 0 Meta/Peer 合同。
4. **P1：降低 write-through 放大**：不改变已确认语义的前提下，为大顺序写设计提交流水线或有界合并；不能偷偷切换成 writeback。
5. **P1：降低跨节点首读放大**：保留 Exact Version + Peer Pull 语义，合并/流式处理连续 Block，并让 ReportReplicas 脱离前台关键路径。
6. 下一轮修改不得改变公开 SDK、DataCore/Meta 职责或 FUSE 语义；必须用本报告的同场 evaluator 防回归。

## 9. 复现入口

先启动三台 VM：

```bash
limactl start g003-n1
limactl start g003-n2
limactl start g003-n3
```

再在源码目录执行采样、汇总与机器判定：

先复制 example profile，并按当前 VM 名称、IP 和实际 `dms-node`/`dms-meta`
release 二进制绝对路径修改其中字段；example 中的占位路径不能直接运行。

```bash
python3 scripts/performance/run_native_vs_moosefs_3vm.py \
  --profile benchmarks/profiles/native-vs-moosefs.example.json \
  --output evidence/native-vs-moosefs/latest/raw

python3 scripts/performance/assemble_native_vs_moosefs_result.py \
  evidence/native-vs-moosefs/latest/raw \
  --output evidence/native-vs-moosefs/latest/result.json \
  --report docs/performance/native-filesystem-vs-moosefs.md

bash scripts/performance/validate_native_vs_moosefs.sh
```
