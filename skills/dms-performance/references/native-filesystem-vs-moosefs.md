# Native Filesystem 与 MooseFS 基线

完整结果见 [`docs/performance/native-filesystem-vs-moosefs.md`](../../../docs/performance/native-filesystem-vs-moosefs.md)，机器结果见
[`evidence/native-vs-moosefs/latest/result.json`](../../../evidence/native-vs-moosefs/latest/result.json)。

## 冻结环境

- 三台相同 Lima aarch64 VM：3 vCPU、约 4 GiB 内存、约 12 GiB 虚拟磁盘。
- A 为 writer/data owner，B 为 remote reader，C 为 metadata service。
- MooseFS goal=1，只有 A 启动 ChunkServer，避免 B 的 peer-first 偶然变成本地读取。
- memory lane：MooseFS Master/Chunk 数据也放 tmpfs，5 轮后端顺序交替。
- disk lane：MooseFS 使用 VM 虚拟磁盘，只展示真实部署差异，不与 DMS 内存可靠性混为一谈。

## 2026-09-17 P0 结论

- DMS 512 MiB 本地热读为 1215.49 MiB/s，高于 MooseFS 803.96 MiB/s；本地 DataCore 路径有明确优势。
- 512 MiB 跨节点接管后的复读吞吐比值为 0.969，达到持平门槛。
- 小文件本地热读仍逐文件产生 `GetFilesystemXattr` 和 `ReleaseFilesystemLockOwner` Meta RPC，违反既有 0 Meta/Peer 热路径合同；这是实现回归，不是架构税。
- 512 MiB write-through 每个 1 MiB callback 同步一次 `CommitFilesystemVersion`，5 轮合计 2560 次，是写吞吐主要放大项。
- 512 MiB peer-first 5 轮产生 2560 次 `PullBlock` 和 2322 次 `ReportReplicas`；分块拉取必要，但逐块 RPC 与副本上报不应直接视为架构下限。
- Preview 判定为 `NOT_READY`。下一轮顺序固定为：恢复小文件热路径合同；优化提交流水线；优化连续 Block pull/report 控制路径。

## P1 后续结论

- 小文件稳定热读已恢复 0 前台 Meta/Peer RPC；两次独立运行 evaluator 均通过。
- local hot 的两轮 p50 比值为 0.741/0.724，peer repeat 为 0.716/0.718；DMS 已稳定快于同环境 MooseFS。
- `GetFilesystemXattr` 和无锁 `ReleaseFilesystemLockOwner` 已从普通读路径删除，没有通过扩大 TTL、关闭 ACL 或跳过 revoke 换性能。
- 下一阶段是 namespace/mutation 固定成本；不要重新优化已经达到停止线的稳定热读。

## P2/P3 后续结论

- P2 已把稳定 stat 收敛为纯本地路径，并冻结 create/patch/create-delete 的最小权威 RPC 合同。
- P3 使用相同 `open -> pwrite -> fdatasync -> close` 调用序列，关闭 writeback，并在三 VM memory lane 完成两次独立 5 轮测试。
- 无 holder 的 1 MiB 同步写两轮吞吐比为 1.108、1.139；本地 payload 不经过中心数据节点的优势已经兑现。
- “文件越大越快”不成立：8 MiB 被拆成 8 个 callback/commit，512 MiB 流式写有 512 个 callback/commit；中心 Meta authority 随发布次数累计后成为架构劣势。
- Meta 单次 commit 内的嵌套扫描属于实现放大，已改为 actor turn 内临时索引；512 MiB Meta business 降低 70.8%～77.3%，但没有改变一次 callback 一次权威发布。
- 远端 holder 每次逻辑写恰好增加一次 invalidation ACK。稳定本地读和 Peer 复读继续 0 前台 Meta/Peer RPC，并优于 MooseFS。
- 推荐 workload 是本地 owner、低 holder 扇出、单次写通常不超过一个 callback、写后重复读取；当前不推荐要求每个 1 MiB chunk 都同步发布的大文件流式写。
- 下一阶段是 P4 Peer 首读；不要把稳定复读、writeback 或高并发问题混入 P4。

## 不要重复争论

- 不要把 peer-first 的全部差距都归因于分布式架构；MooseFS 同样需要定位和远端 payload。
- 不要用 DMS memory 对 MooseFS disk 的数字宣称公平领先。
- 不要先优化 memcpy：大文件本地热读和 peer-repeat 已证明 payload 路径接近目标。
- 不要通过暗改 write-through 为 writeback 获得吞吐；写回语义必须另行设计和批准。

机器判定：

```bash
bash scripts/performance/validate_native_vs_moosefs.sh
```
