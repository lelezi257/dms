# Ordinary OwnerFs Performance Criteria

2026-10-07 user decision. This page is the current source-side authority for ordinary OwnerFs read/write performance targets. It updates active G2 acceptance wording only; historical evidence keeps its original version, criterion and verdict.

## Plan For This Criteria Change

Update active goal, acceptance, execution and status documents to point to this rule. Do not rerun benchmarks just for the wording change. Reuse only evidence that already has the same comparator, operation, cache, durability and latency fields; otherwise keep the result as diagnostic or pending.

## Execution Priority

Required usage enables workspace bind ON. First close the finite ON functions and scenario delivery; then remote FUSE cooperation/read-write performance; then DFS one-writer/many-readers; ordinary local FUSE keeps the same dual targets but executes later. Existing gaps justify bounded product optimization, one main issue per round, with same-condition before/after metrics and correctness. Tool readiness and qualification work are not product completion.

## Current Rule

Ordinary OwnerFs local and remote core read/write cases use MooseFS as the comparator.

| Metric | Acceptance rule |
| --- | --- |
| Throughput | The paired median AFS throughput must be >= 1.2 x the paired median same-condition MooseFS throughput. Publish every paired run value. |
| Operation latency | The predeclared judging percentile for AFS operation latency must be <= 0.8 x the same percentile for MooseFS latency. Default judging percentile is p95. |

Throughput and latency are judged independently. A case passes only when both requirements pass for the same frozen case. Throughput uses paired medians by default; latency uses the predeclared percentile over a per-operation sample array. Do not infer latency from throughput, and do not use a favorable throughput result to waive a latency miss.

OwnerFs workspace bind mount keeps its separate target: approach native ext4 for the explicitly enabled bind path, with the switch default OFF. DFS keeps its separate target: match 3FS under the same POSIX/FUSE interface and three synchronous durable copies. Delete cases keep correctness plus comparative performance reporting, with no added hard ratio.

## Case Freeze Before Running

Each ordinary OwnerFs performance case must freeze these fields before it starts:

- Interface and mount path: FUSE/POSIX path, local Home or remote Home, and comparator mount.
- Dataset: size, block size, file count, seed and content validation method.
- Concurrency: task count, worker count and client placement.
- Cache state: cold, repeated buffered, or proven hot; unproven cache state is diagnostic only.
- Durability boundary: close-only, fdatasync or fsync, with the same visible/durable state on both systems before timing ends.
- Replica and backend semantics: OwnerFs Home/local-file behavior and the MooseFS goal and write acknowledgement mode.
- Capacity and stop budget: data volume, raw evidence budget and required free-space floor.
- Timing boundary: choose before execution whether operation latency means a file transaction (`open/create -> I/O loop -> selected barrier -> close`) or per-I/O call (`read`/`write` syscall or equivalent probe interval). The choice must be identical for AFS and MooseFS and cannot be changed after seeing results.
- Sample method: timer clock, sample count, warmup count, paired order and quantile method are fixed before execution.
- Judgment percentile: p95 is the default judging percentile unless the case plan declares another percentile before execution.

Every run records throughput plus p50, p95 and p99 for the measured operation-latency sample array. Read file-transaction timing covers `open -> reads -> close` after the file is prepared and verified. Write file-transaction timing covers `open/create -> writes -> selected barrier -> close` and ends only after the chosen visibility/durability condition matches the comparator. Existing C aggregate wall-time divided across a task summary is throughput diagnostic evidence, not syscall or per-operation latency evidence.

**当前出口（2026-10-08，G2.15）：** 远端WRITE单次64MiB/C1摸底留数：Home/client写RPC均值0.107/2.420ms，Home占4.42%；fdatasync47.687ms，占整轮12.85%，不弱化屏障。原读文件不变、新写内容/权限/EOF及三actualwait0已核对；B固定256MiB目录预算因漏算新增文件超13,185,024B，仍有21.32GB磁盘余量，**全轮FAIL_BUDGET保留，不计性能通过、不重跑**。32保存数据审计和242raw+27guest实际Linux恢复通过，只证明相应证据。[纯Owner节点TCP入站帧候选](evidence/20261008-owner-remote-write-server-frames/README.md)4项受影响Rust回归、check及限定Clippy通过，补丁实际恢复可用；尚无性能对照，未保留到生产，160输入已恢复原READ版本。正式写对照未启动，夹具空间处理选择待答复。[写摸底与失败](evidence/20261008-owner-remote-write-rpc-cost/README.md)。既有[READ吞吐+14.21%/独立p95-4.29%改善](evidence/20261008-owner-remote-read-frames/README.md)及b80 ON包身份不变；正式Moose1.2/.8仍待验。DFS写按用户选择暂缓，G1历史8/8关闭、原G2仍12/0/15，不把诊断或代码检查算产品完成。

## Existing Evidence Audit

| G2 item | Existing evidence | New-criterion status |
| --- | --- | --- |
| G2.09 local read | e925c5b 64MiB/C1 data was compared with ext4 under the old rule; dev ratio 0.5709 x ext4, release ratio 0.3477 x ext4. [Evidence](evidence/20261006-e2e-current/r4/README.md). | Pending under the new MooseFS rule. No same-condition MooseFS local baseline and no predeclared latency percentile evidence. Old FAIL remains historical. |
| G2.10 local write | e925c5b 64MiB/C1 fdatasync data was compared with ext4 under the old rule; dev ratio 0.6297 x ext4, release ratio 0.5879 x ext4. [Evidence](evidence/20261006-e2e-current/r4/README.md). | Pending under the new MooseFS rule. No same-condition MooseFS local baseline and no predeclared latency percentile evidence. Old FAIL remains historical. |
| G2.14 remote read | 6d B-to-Home 64MiB/C1 data recorded AFS 427.371 MiB/s and MooseFS 14854.399 MiB/s, ratio 0.028803; cache state and MooseFS client cleanup were not fully qualified. [Evidence](evidence/20261007-owner-remote-small/README.md). | Pending under the new rule. Existing independently timed per-operation p50/p95/p99 summaries are retained. Raw interval arrays and a pre-run judging percentile are absent; cache/cleanup qualification is incomplete. These historical summaries are diagnostic, not current f03 timings. [Reuse audit](evidence/20261008-owner-remote-latency-reuse/README.md).  当前f03已留5测量/320实际区间：396.945760MiB/s、p95=3125040ns；Moose DIRECT观察mmap ENODEV、零参考样本，无配对/无比率，仍待验。原失败和wait0留证，兼容性停止单列；下一独立bind功能项。[本轮](evidence/20261008-owner-remote-read-current/README.md)。 |
| G2.15 remote write | 6d B-to-Home 64MiB/C1 data recorded AFS 234.630 MiB/s and MooseFS 430.767 MiB/s, ratio 0.528615; MooseFS strong durable-ACK baseline remains blocked. [Evidence](evidence/20261007-owner-remote-write-small/README.md). | Pending under the new rule. Existing independently timed per-pwrite p50/p95/p99 summaries are retained, with barrier time separate. Raw interval arrays, pre-run judging percentile and durable comparator qualification are incomplete. [Reuse audit](evidence/20261008-owner-remote-latency-reuse/README.md). |


Current f03 local read now has independently measured raw latency arrays and a frozenp95, but its formal hot prerequisite failed before timing. The separately frozen repeat diagnostic has unequal client residency (Owner0/Moose64MiB); ratios andp50/p95/p99 are retained as diagnostic, not formal PASS. [Case evidence](evidence/20261008-owner-local-read-latency/README.md). The missing item is matched-cache qualification, not a blanket lack of any latency samples.

Current f03 local create+fdatasync write also retains independent raw latency and separate barrier times. Twelve fresh 64MiB files passed core content/EOF checks, but MooseFS strong durable-ACK and backing-cache comparability remain unqualified. The measured ratios are diagnostic, not a formal pass or an accepted relaxation. [Write evidence](evidence/20261008-owner-local-write-latency/README.md).

Stage G1 remains closed at historical 8/8. Current-version regressions, ordinary OwnerFs performance under this rule, and missing latency evidence belong to G2.

## Current Backend-hot / Default-buffered-client Read

New prospectively frozen G2.09 case, not a relaxation or retrospective change to old client-hot/reread contracts. Both FUSE/POSIX/O_RDONLY applications use identical full client and physical-payload preload, with physical64MiB residency and stable identity required before/after each timer. Internal default client policies remain Owner OFF/private-local eligibility and Moose AUTO; observed client0/64MiB is reported, not claimed identical.

Fixedf03/7bfc limited comparison **FAIL**: median throughput ratio0.345207<1.2, independently pooledp95 ratio3.046710>.8, each320raw intervals. Backing24snapshots/content/actualwaits and independentLinux audit passed. Old client-hotFAIL/repeatNOT_QUALIFIED remain. This provides a qualified failure only for the newly declared backing-hot/default-application policy; it does not cover all cache regimes. [Exact contract, raw data and scope](evidence/20261008-owner-local-read-backing-hot/README.md). No target/caching change after results, no immediate tuning or unchanged repeat.

## Current remote backing-hot / default-client read

G2.14 independent, prospectively frozen f03 case: A reader/B Home and sole VALID Moose copy/ctl local-file;64MiB/C1/1MiB. Same POSIX/O_RDONLY and full client+physical preload;24 physical before/after snapshots hot and stable. Internal defaults are disclosed separately: Owner client0 versus Moose AUTO64MiB; no equal network traffic/end-to-end-residency claim.

Median throughput ratio0.026616<1.2 and independently pooled nearest-rank p95 ratio28.332518>.8: **both FAIL**. Each320 raw measured intervals retained; old DIRECT/ENODEV failure is a separate stopped case, never retrospectively qualified. Content and normal closure passed; this read result is not strong write-durable qualification. [Contract, data, tool errors and full scope](evidence/20261008-owner-remote-read-backing-hot/README.md). Point tuning deferred, targets unchanged.
