# Ordinary OwnerFs criteria synchronization

2026-10-07 user decision; documentation/manifest change, no benchmark rerun.
[Current criteria](../../ownerfs-performance-criteria.md), [goal table](../../trial-release-goals.md), [acceptance](../../../docs/acceptance.md).

Local and remote ordinary core read/write each require throughput >=1.2x comparable MooseFS AND independent operation latency <=0.8x. Report p50/p95/p99; decision percentile, operation/timer boundary, sample count and quantile method frozen before each case. Workspace bind/ext4, DFS/3FS and deletion report targets unchanged. G1 historical8/8 stays closed; current supplements G2.

G2.09/10/14/15 are PENDING under the new rule: local e925 ext4 results have no matching MooseFS baseline; remote6d data has cache/comparator or durable-ACK qualification gaps; all four lack predeclared independent operation latency evidence. Old version/criteria/FAIL/BLOCKED remain historical, never translated into new PASS or new measured FAIL. Current G2 count11 limited outputs/1 bind function in progress/15 pending.

Linux manifest check passed:70 entries (69 active plus reserved REL-15), only PERF01..04 criteria/evidence wording changed; case IDs, matrices, driver readiness, statuses and other cases unchanged. Frozen acceptance.lock and protected handoff untouched. Rust inputs/ELFs and current DFS E2E candidate unchanged; no Rust build needed. Remaining new latency collection and qualified comparisons proceed as small G2 tasks, not immediate full-matrix runs.

The native active Goal tool exposes status-only updates and cannot replace its old objective string in place. Goal remains ACTIVE; this latest user instruction and the current goal table supersede its historical criterion text. Do not complete/recreate the whole goal just to change wording. Protected handoff also retains its original snapshot; current criteria take precedence.

[Project tracker](https://chatgpt.com/space/page_8052a31cd4f4819184ea7da223793b7f) updated and final read verified; all11 original template blocks unchanged. A topology-changing literal patch was rejected without commit; a supported insertion succeeded, then an item-number correction was applied. Full Page snapshots and exact commands remain outside Git with SHA index, no script snapshots copied into this packet.
