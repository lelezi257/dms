# Delivery task map

[Acceptance](../docs/acceptance.md) is the release contract. Execute the following dependency stages; detailed design and evidence are prepared per task. Dynamic checkpoints, raw results and significant-change records are execution artifacts, not product architecture documentation.

## Work breakdown

| Task | Cases | Design/development work | Exit evidence |
| --- | --- | --- | --- |
| P0.1 Environment | ENV-01 | Freeze image/kernel, dedicated CIDR, resources, state/data volumes, RXE, TLS, isolated build runtime; preserve old VM data | lock, inventory, probes, exact identities |
| P0.2 Suites | STD-01..05 | Prepare upstream suites, explicit applicability/accounting, ext4 reference, differential runner, result schema | case manifest, raw reference results, no missing tests |
| P0.3 Baselines | ENV-01, PERF-01..08 | Actual ARM64 MooseFS/3FS/FDB build/mount/RXE; immutable fair config and datasets | successful mounts, exact hashes, comparison readiness |
| P0.4 Runner/Skill | OPS-06 | Reusable stages/results, fault injection/verifier and evidence collection; no false PASS | self-check against known failure, invocation guide |
| P1 Installable vertical slice | FUN-01/03/04/13, DEP | Package/process lifecycle plus R1 Owner/DFS smoke; Linux build/check/matrix | clean guest create/write/sync/close/reopen |
| P2 Backend/semantics | FUN-02..12, STD, DIST-01/02/06/07, REL-01/02/05/06/07/09/10/11/12 | same-mount kernel cache, close barrier, inode queue/unknown replay, remote owner, permissions/namespace/locks/mmap; Redis durable snapshot backend parity | targeted regressions then full applicable short matrices |
| P3 Replication/transport | DIST-03..05/08, REL, RDMA | synchronous RN and durable async tasks/repair; placement epochs, authorization, coherent source retry; integrated inline/gRPC/RDMA data paths | real multi-VM and fault evidence, R1 zero-peer proof |
| P4 Storage/observability | REL-03/04/08/13/14, OPS-01..07 | streaming bounded memory, crash/reconcile/GC, ENOSPC/corruption, metrics/traces/readiness/Home status | recoverability and fault diagnosis, resource accounting |
| P5 Performance | PERF-01..08, DIST-07 | profile locked short workloads, then optimize real RPC/copy/serialization costs within contracts | validated paired results meeting every threshold |
| P6 Full delivery | ALL | Complete POSIX matrices, 8GiB/900s seeds/long-run, package/offline/idempotency/restart, independent architecture/code review | zero unresolved mandatory gates; installable artifacts; final change review |

P1/P2/P3 are vertical slices, not permission handoffs. Add smaller dependent tasks when code inspection identifies missing design, preserving the target. P0 prerequisites constrain performance tuning; independent functionality work may proceed while external downloads build. Do not count ENV-01 complete before reference mounts/probes/suites work.

Use the [impact-based validation strategy](validation.md#feedback-stages) within each task. A short exit allows development to advance; it is not a full source or formal acceptance gate. Complete the source gate at stage/batch boundaries, reuse unchanged qualified inputs and keep full POSIX, 8 GiB, performance matrices and long stability tests in their scheduled acceptance stages.


## Detailed dependent tasks and short exits

In the research workspace, evidence uses `evidence/afs-delivery/<task>/<run-id>/` with commands, identity.json, result.json, raw logs and a minimal failed operation sequence. `experiments/afs-acceptance/cases.json` owns case applicability, drivers and smoke/full distinction; `acceptance.lock.json` owns exact environment. Keep placeholders NOT_RUN until executed. Runtime results use only contract-approved result statuses.

| Task | Prerequisites | Focused design / implementation | Short exit before advancing |
| --- | --- | --- | --- |
| P0.2a | acceptance | case manifest schema, driver bindings, result identity and suite accounting | 69 distinct active IDs, reserved REL-15 excluded, injected missing-test/failure makes runner fail |
| P0.2b | P0.1/P0.2a | pin upstream tests, inventory all discovered TAP/LTP cases and applicability | ext4 reference passes or each reference/environment discrepancy explained; no newly invented exclusions |
| P0.3a | P0.1 | stock comparator durability audit/config and ARM dependency budget | reference ACK/barrier durability shown or specific lane remains BLOCKED |
| P1a | Linux build runtime | reproducible build/features/package and dependency manifest | package can be installed without cargo/git on clean guest |
| P1b | P1a/runtime volumes | minimum DEP-01/02 process and mount deployment | Owner/DFS independent mounts, etcd-backed create/write/sync/close/reopen with content check; not full DEP approval |
| P2a | R1 code tests | close-flush commit, dup/fork watermarks, exact pending replay and errors | FUN-03/05 and REL-04/11; close-only durable reopen, no empty extra versions, error reaches app |
| P2b | P2a | same-mount dirty read view, file length/getattr and FUSE cache policy | FUN-02 with previously open readonly fd on real kernel cache, override/append/resize locally and remotely |
| P2c | P2a/P2b | namespace atomicity/open-unlink, permissions/xattr/time, owner-lock scope, mmap writeback, directory persistence | FUN-07..11/REL-13 plus relevant pjdfstest/LTP subsets; design each missing interface before adding it |
| P2d | P2a/P2b | remote DFS inode-owner write/resize/sync/flush authorization, queue and completion | DIST-02 and remote FUN-02/03/04; stale owner denied; no need to force all remote readonly handles through dirty view |
| P2e Redis | store abstraction | choose existing opaque full-snapshot StoreBackend boundary, expected-version atomic CAS and exact unknown-outcome replay; probe dedicated AOF always/noeviction/no TTL; backend parity | same store contract tests on etcd/Redis; ACK-loss replay, AOF restart/rewrite interruption and backend failure prove visible state only after durable ACK |
| P3a | P2 owner semantics/placement | R1 local zero-peer invariant then RN gRPC staging/finalize/receipts | R=1/2/3 short writes, distinct-node epochs/receipts, commit after required replicas, no partial publication |
| P3b | P3a/read authorizer | fixed-version read grant, real peer-read authorization and range completeness/retry | DIST-04/05, corrupt/short source and concurrent new version do not leak scratch/mixed bytes |
| P3c | P3a/store durability | persistent async repair tasks, source-loss/degraded status, replacement device/placement | DIST-08/REL-07, failed worker/restart preserves task; no live sources reports unavailable/BlockedNoSource immediately, without declaring permanent loss |
| P3d | P3a/P3b + cross-VM RXE | common descriptor/completion identity, MR/QP/CQ lifetimes, gRPC control with RDMA payload, Owner/Home and DFS parity | RDMA-01..05 short files/edges and failure injection; actual verbs bytes with file trace, required mode refuses fallback |
| P4a | P2/P3 | bounded streaming dirty/staging, local finalize/recovery/orphan/GC, capacity and digest checks | small chunk-boundary and constrained-volume cases cover later 8GiB shape; no data-dependent whole-file RAM |
| P4b | each vertical slice | health/metrics/trace/diagnostic/REST Home availability/backpressure | OPS-01..07, request can be traced to durable boundary, unavailable Home not healthy, bounded labels/resources |
| P5 | ENV-01 valid baselines + functional slices | profile short fixed workloads, preserve contracts, optimize proven costs | no correctness regression; exact RPC/copy/resource evidence before full perf runs |
| P6 | previous tasks | all mandatory matrices, full STD seeds, REL-14 8h, 8GiB workloads, all DEP-01..08, independent review | final manifest coverage/accounting, paired 5-run per workload targets, reproducible release and final architecture change summary |

Design alternatives to evaluate only where code needs extension: reuse atomic full-snapshot backend rather than adding record-transaction dependencies; reuse inode owner queue rather than operation log/next dirty batch; implement transport-independent replica completion first rather than two divergent gRPC/RDMA business state machines. Required auth/cache/POSIX changes must use existing FUSE/Node/RPC boundaries; significant alternatives enter changes.md with evidence.
