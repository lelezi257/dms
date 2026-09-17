# Native Filesystem P2：Namespace 与 Mutation 固定成本收口

> 结论：**PASS（延迟门槛或分段证明）**。两次独立三 VM 运行使用相同源码和二进制。稳定 stat 已成为纯本地路径；三项 mutation 的重复控制请求已经删除，剩余成本由当前 POSIX/write-through/强一致合同决定，并由同轮白盒分段覆盖 86.6%～87.7%。

## 1. 本阶段解决的问题

P2 不把分布式权威 mutation 假装成本地内存操作。它回答三个问题：

1. 一次用户操作是否只产生理论上必要的 Meta authority 请求。
2. 缓存失效是否只发给真实持有者，且只等待这些持有者 ACK。
3. 延迟仍未达到对端时，剩余时间是否能由真实分段解释，而不是隐藏的重复 RPC。

## 2. 实现变化

| 变化 | 原因 | 结果 |
| --- | --- | --- |
| namespace mutation 回复附带变更目录快照 | Node 已经拥有最新 parent attrs，不应提交后重新查询 | create/remove/rename 后直接刷新本地目录镜像 |
| 一个 Watch event 携带同一 mutation 的全部 inode | 目录和目标 inode 属于同一权威提交，不应拆成多轮通知 | 一次投递、一次 ACK 完成精确失效 |
| Meta 记录每个文件 grant 的真实 holder | active Node 不等于缓存持有者 | 无关 Node 不接收事件，也不阻塞提交 |
| 恢复路径保持保守 fallback | Journal 旧记录可能没有 holder snapshot | 缺少精确信息时仍优先保证一致性 |
| 本地已有 xattr 状态时直接拒绝缺失项 | 缺失 xattr 不是再次访问 Meta 的理由 | mutation 热路径消除重复 GetXattr |

## 3. 双轮端到端结果

| 独立运行 | Case | DMS p50 | 对端 p50 | 比值 | DMS mean | 正确性 |
| --- | --- | ---: | ---: | ---: | ---: | --- |
| run-1 | `workspace.stat` | 2.875 µs | 169.299 µs | 0.017 | 3.212 µs | PASS |
| run-1 | `workspace.create` | 1283.176 µs | 870.914 µs | 1.473 | 1298.625 µs | PASS |
| run-1 | `workspace.patch` | 888.329 µs | 649.651 µs | 1.367 | 904.632 µs | PASS |
| run-1 | `workspace.create_delete` | 1806.571 µs | 1056.044 µs | 1.711 | 1807.522 µs | PASS |
| run-2 | `workspace.stat` | 2.792 µs | 158.005 µs | 0.018 | 3.101 µs | PASS |
| run-2 | `workspace.create` | 1246.079 µs | 813.817 µs | 1.531 | 1256.300 µs | PASS |
| run-2 | `workspace.patch` | 886.110 µs | 661.395 µs | 1.340 | 904.796 µs | PASS |
| run-2 | `workspace.create_delete` | 1763.347 µs | 1047.700 µs | 1.683 | 1772.341 µs | PASS |

这些数字没有被描述成“全面快于对端”。stat 已兑现本地缓存优势；mutation 仍有分布式权威提交成本，P2 通过白盒合同判断它是必要成本还是实现放大。

## 4. 冻结后的最小 RPC 合同

| Case | 每样本前台 RPC | 为什么存在 |
| --- | --- | --- |
| `workspace.stat` | 0 | 有效 grant 内直接读取 Node 本地 inode snapshot |
| `workspace.create` | 1× Lookup + 1× Create + 1× Commit | VFS negative lookup、namespace authority、随后独立到达的 FUSE write callback |
| `workspace.patch` | 1× Commit + 真实 holder 的 1× ACK | write-through 发布后，远端旧 binding 必须先失效才可返回 |
| `workspace.create_delete` | 1× Lookup + 1× Create + 1× Commit + 1× Remove + 1× orphan reference release | create/write/unlink 是独立 POSIX mutation；孤儿 release 驱动及时回收 |

`GetFilesystemInode`、`GetFilesystemXattr`、`ReleaseFilesystemLockOwner`、`ResolveObject`、`PullBlock` 在这些前台路径中均为 0。无关但 active 的 Node 不产生 ACK。

## 5. 同轮分段账本

| 运行 | Case | E2E mean | 已解释 | 覆盖率 | kernel/FUSE/syscall 残差 |
| --- | --- | ---: | ---: | ---: | ---: |
| run-1 | create | 1298.625 µs | 1129.814 µs | 87.0% | 168.812 µs |
| run-1 | patch | 904.632 µs | 789.922 µs | 87.3% | 114.710 µs |
| run-1 | create/delete | 1807.522 µs | 1585.272 µs | 87.7% | 222.250 µs |
| run-2 | create | 1256.300 µs | 1088.192 µs | 86.6% | 168.109 µs |
| run-2 | patch | 904.796 µs | 790.823 µs | 87.4% | 113.973 µs |
| run-2 | create/delete | 1772.341 µs | 1549.793 µs | 87.4% | 222.548 µs |

create 的主要段是 lookup/create/write；patch 的主要段是 write，且包含真正需要等待的远端 revoke ACK；create/delete 再增加 unlink 与孤儿 reference release。没有把嵌套在 operation 内的 RPC duration 重复相加。

## 6. 验收器为什么允许分段证明

原始 Preview 目标要求三项 mutation 相对 P0 p50 至少改善 50%，且相对对端不高于 1.25。实际结果没有全部达到，所以验收器没有降低数字或把失败隐藏掉，而是实现合同中预先约定的第二条出口：

- RPC 次数、forbidden RPC、正确性、两轮独立身份仍是硬门禁，任何一项失败都不能由分段证明覆盖。
- 只有同轮必需分段覆盖 mean 至少 85%，才接受“剩余为明确架构/语义成本”的结论。
- 输出同时保留原始 latency failure 和每段耗时，后续阶段仍可继续优化单跳效率。

机器入口：

```text
bash scripts/performance/validate_native_fs_namespace_mutation.sh
```

## 7. 停止线与下一步

P2 在这里停止，因为继续删除 RPC 会破坏 namespace authority、write-through 或强一致失效语义。下一项是 P3：建立等语义 write-through lane，分解并优化一次 Commit 内的数据准备、Journal、ACK 和缓存回填。gRPC/Actor 单跳固定成本属于 P5 的并发与 transport 上限，不在 P2 继续混调。

源码身份：`d794a6d31eabea35f00e1d30ff3bc4a8ff8677d0`。两轮 `dms-node`/`dms-meta` 二进制哈希一致；精简机器基线为 [`native-fs-namespace-mutation-lima-aarch64-2026-09-17.json`](../../benchmarks/whitebox/baselines/native-fs-namespace-mutation-lima-aarch64-2026-09-17.json)，原始机器结果位于 `evidence/native-fs-namespace-mutation/run-1` 与 `run-2`。
