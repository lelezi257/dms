# Ordinary OwnerFs Performance Criteria

2026-10-07 user decision. This page is the current source-side authority for ordinary OwnerFs read/write performance targets. It updates active G2 acceptance wording only; historical evidence keeps its original version, criterion and verdict.

## Plan For This Criteria Change

Update active goal, acceptance, execution and status documents to point to this rule. Do not rerun benchmarks just for the wording change. Reuse only evidence that already has the same comparator, operation, cache, durability and latency fields; otherwise keep the result as diagnostic or pending.

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

## Existing Evidence Audit

| G2 item | Existing evidence | New-criterion status |
| --- | --- | --- |
| G2.09 local read | e925c5b 64MiB/C1 data was compared with ext4 under the old rule; dev ratio 0.5709 x ext4, release ratio 0.3477 x ext4. [Evidence](evidence/20261006-e2e-current/r4/README.md). | Pending under the new MooseFS rule. No same-condition MooseFS local baseline and no predeclared latency percentile evidence. Old FAIL remains historical. |
| G2.10 local write | e925c5b 64MiB/C1 fdatasync data was compared with ext4 under the old rule; dev ratio 0.6297 x ext4, release ratio 0.5879 x ext4. [Evidence](evidence/20261006-e2e-current/r4/README.md). | Pending under the new MooseFS rule. No same-condition MooseFS local baseline and no predeclared latency percentile evidence. Old FAIL remains historical. |
| G2.14 remote read | 6d B-to-Home 64MiB/C1 data recorded AFS 427.371 MiB/s and MooseFS 14854.399 MiB/s, ratio 0.028803; cache state and MooseFS client cleanup were not fully qualified. [Evidence](evidence/20261007-owner-remote-small/README.md). | Pending under the new rule. Existing data remains diagnostic because comparator qualification and latency percentile evidence are incomplete. |
| G2.15 remote write | 6d B-to-Home 64MiB/C1 data recorded AFS 234.630 MiB/s and MooseFS 430.767 MiB/s, ratio 0.528615; MooseFS strong durable-ACK baseline remains blocked. [Evidence](evidence/20261007-owner-remote-write-small/README.md). | Pending under the new rule. Existing data remains diagnostic because durable comparator qualification and latency percentile evidence are incomplete. |


Current f03 local read now has independently measured raw latency arrays and a frozenp95, but its formal hot prerequisite failed before timing. The separately frozen repeat diagnostic has unequal client residency (Owner0/Moose64MiB); ratios andp50/p95/p99 are retained as diagnostic, not formal PASS. [Case evidence](evidence/20261008-owner-local-read-latency/README.md). The missing item is matched-cache qualification, not a blanket lack of any latency samples.

Current f03 local create+fdatasync write also retains independent raw latency and separate barrier times. Twelve fresh 64MiB files passed core content/EOF checks, but MooseFS strong durable-ACK and backing-cache comparability remain unqualified. The measured ratios are diagnostic, not a formal pass or an accepted relaxation. [Write evidence](evidence/20261008-owner-local-write-latency/README.md).

Stage G1 remains closed at historical 8/8. Current-version regressions, ordinary OwnerFs performance under this rule, and missing latency evidence belong to G2.
