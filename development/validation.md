**2026-10-08 当前新增（事实/决策）：** 固定f03/7bfc复用停止的local-file/R3夹具，仅Meta正常重启，三Node/挂载/UDS身份不变、重启前A/B/C全64MiB SHA/EOF与48物理份通过，五actualwait0/12保护进程和预算闭合。原观察器重启后把同三节点跨epoch六历史回执计成六副本而FAIL，后续物理检查/读回NOT_RUN；post-closure实体及代码支持观察器判据不匹配，不称产品六物理副本或恢复PASS。Linux6独立guards/137保存数据检查、600raw/51guest及工具/失败脚本实际恢复通过；所有首失败保留，无产品修补/重跑/环境变更。G1历史8/8、G2总数/defaultOFF不变。下一独立小项：副本观察器跨epoch唯一serving-node/设备/catalog-floor/失效authority针对性覆盖，再补受影响R3正常恢复读回；f03 R1和旧标准直接复用，7e6 R3保持历史，点优化/3FS资格/复杂可靠性后置。 [版本、FAIL及证据](evidence/20261008-dfs-r3-meta-recovery/README.md)。 以下保留原时点。

# Validation strategy

Validation follows [the three-stage acceptance checklist](trial-release-goals.md). The purpose is to make progress in independently reviewable units: first runnable, then standard and small performance cases, then broad reliability and backend matrices.

All Rust builds, filesystem tests, privileged FUSE runs and product runtime checks must run on the ARM64 Linux VM environment. macOS is allowed for editing, Git operations, documentation review and VM orchestration only.

## Evidence levels

| Level | Use | Minimum evidence |
| --- | --- | --- |
| Static/document check | Documentation, manifests and non-runtime scripts | Link check or grep-based consistency check, plus `git diff --check` |
| Unit/contract check | Narrow Rust or Python behavior change | Targeted test for the changed behavior, formatting/lint when applicable |
| Source gate | Shared Rust behavior, protocol, FUSE, Meta, Node or deployment changes | Linux format, strict Clippy/build and affected library/contract/integration tests |
| Runtime smoke | Candidate usability or affected distributed behavior | Identified binaries/configs, real mount/process identities, raw commands, content checks and cleanup |
| Standard suite | POSIX fallback and regression safety | pjdfstest, fixed LTP subset and short FSx with full accounting and predeclared exclusions |
| Performance case | G2 performance item | Baseline and candidate on the same frozen case, correctness proof, resource identity, raw timing, noise policy fixed before measurement |
| Formal release gate | G3/final release | Full suite matrix, 8 GiB cases, long soak, comparator qualification, durable backend/fault/RDMA/deployment evidence |

Short smoke success never replaces a full case. A full old run does not qualify a changed candidate. Each result must bind source, binary/package, command, environment, mount identity, raw output and pass/fail criteria.

## Stage-specific gates

### G1 retained trial

G1/g1.5 remains complete in its historical scope: OwnerFs/DFS installable trial, memory demo and central local-file Meta restart recovery. Do not use new candidate failures to erase that result. Do not upgrade it into full POSIX, formal 69-case or performance qualification.

### G2 current work

G2 starts with Owner-first standard fallback and small cases, then enters DFS standard checks before DFS performance:

- OwnerFs pjdfstest on the current candidate.
- Owner-relevant fixed LTP filesystem/permission/lock subset.
- Owner-relevant short fixed-seed FSx.
- Affected Owner basic operation and local-file recovery combination.
- Performance priority: container-mounted workspace access from Issue42/PR43, with explicit-switch/default-OFF functional qualification before paired OFF/ON/ext4 measurements.
- Ordinary Owner local/remote and DFS cases may baseline and retain raw data; defer targeted tuning unless the active item needs it. Ordinary Owner read/write performance now follows [the current OwnerFs criteria](ownerfs-performance-criteria.md): throughput >=1.2x same-condition MooseFS and independently measured operation latency <=0.8x MooseFS. Unchanged passing standard results are reused with their version/scope, not rerun as performance tests.
- DFS pjdfstest, DFS-relevant LTP/FSx and DFS affected basic checks before DFS performance claims.
- DFS one-writer/many-readers before broader DFS performance.

Performance cases start small, defaulting to 64 MiB unless the item says otherwise. 512 MiB and 8 GiB are separate records. Large data, long-running and complex mixed cases do not block smaller completed items.

### G3 deferred gates

G3 contains broad LTP/POSIX, long FSx/differential random, 8-hour soak, failure matrices, RDMA abnormal lifecycle, multi-Meta/HA, wider deployment, etcd resource topic and Redis. Keep any local evidence, but do not claim these gates until the full case exits pass.

## Comparator rules

Ordinary OwnerFs local and remote read/write use MooseFS as the comparator and require both throughput >=1.2x and operation latency <=0.8x under the same frozen case. OwnerFs workspace bind mount keeps the separate native-ext4 ON/OFF comparison. DFS targets 3FS parity under matched POSIX/FUSE and three synchronous durable copies. Delete cases require correctness and a measured comparative report; they have no new hard ratio unless a later checklist item adds one.

Baseline and candidate must use the same interface, data shape, concurrency, cache policy, durability barrier, applicable replica semantics, mount type and resource budget. Noise tolerance, timer clock, sample count, quantile method and latency judging percentile are fixed before the run; throughput defaults to paired-median comparison, p50/p95/p99 are recorded from a per-operation latency sample array, and latency is not inferred from throughput. Do not rerun an unchanged failed case until a new input or hypothesis exists.

## Capacity and logs

Capacity admission is per case. Small standard and 64 MiB performance cases should run before 8 GiB or long soak. A case that needs more disk must state the live data, baseline/candidate order, cleanup plan, log budget and host free-space requirement. Expanding a VM disk is allowed after protecting current services and state, but it is not a prerequisite for the first small cases.

## Reporting

Every report must separate:

- PASS/FAIL/BLOCKED/INCONCLUSIVE.
- Current candidate evidence from historical evidence.
- Product evidence from tooling or environment preparation.
- Memory Meta evidence from local-file, etcd and Redis persistence.
- OFF/FUSE evidence from bind/native ON evidence.

If a result is blocked by environment, record the blocker and continue with independent items that do not depend on that environment.

## Container workspace small diagnostic

The [fixed diagnostic slice](container-workspace-perf-slice.md) restores the two first-party C payloads by fixed Git identity, builds only on Linux, and times actual OCI-process work through OFF/FUSE or controlled ON exec. One warmup/five alternating paired rounds retain content, exact argv, namespace/source, live ELF and cleanup evidence. [Current data](evidence/20261007-container-perf/README.md) is diagnostic only; unobserved cache and unavailable exact FUSE counts remain limits, and failed mixed semantics block G2.13 qualification. Do not repeat this case merely to polish noisy small measurements.
