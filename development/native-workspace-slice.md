**Current test boundary (2026-10-07):** The workspace observer is now an explicit Cargo example from `tests/support/workspace_probe.rs`; the experimental adapter requires configured `idle_command` and `identity_command`. The observer JSON protocol remains a runtime safety requirement. Core/host bind has no observer dependency. [Scoped plan, build and runtime evidence](workspace-probe-remediation.md) records the changed candidate separately from historical results below.

**新增当前限定通过（2026-10-07，事实）：** main产品1451f60/map196a，复用已通过Linux构建，两个现有ELF包逐字节一致；当前Node受管workspace成功启动/权限errno双视图/正常停止通过，50驱动检查及30独立postcheck通过，Node/Meta实际wait0、四服务/监督PID及容器消失、保护身份不变。标准/性能/重启未重跑；G1历史8/8、G2计数和defaultOFF不变。宿主独立开关/通用排空及混合语义缺口仍未完成；下一既定workspace限定性能小项。 [版本与证据](evidence/20261007-ownerfs-bind-node-accepted/README.md)。

# Runc adapter for OwnerFs workspace bind mount

2026-10-07; baseline main `0891cbfe558cdda7c8d780b7fa9e8f97329e2554`, mechanism reference [Issue42](https://github.com/lelezi257/dms/issues/42) / [PR43](https://github.com/lelezi257/dms/pull/43) head `80b0bca3d9d86a1357aa745bb65abfb567f5623f`.

**Decision:** pjdfstest qualifies functional completeness. The first performance lane is container-mounted workspace access. Ordinary FUSE/local/remote/DFS timings and failures remain available for later focused optimization. This slice does not reopen historical G1 8/8 or change previous acceptance conclusions.

## Current module boundary (2026-10-07)

The shared mount/identity/normal-unmount core now belongs to one `src/node/vfs/ownerfs/bind_mount.rs` file, with `WorkspaceBindMount` and component-parameterized secondary-clone operations. This file remains the runc adapter design entry; its historical path and evidence references are preserved. The physical Home source is covered onto the FUSE first-level workspace only in the controller private namespace; ordinary callers have no covering bind. Core extraction is not a new generic Node lifecycle or full bind acceptance. Existing TOML/CLI adapter options retain names/semantics/default OFF. [Independent repair plan](ownerfs-workspace-bind-remediation.md).

## Implemented boundary

An explicitly enabled, administrator-only experiment connects Node to one managed Home workspace container. Default OFF uses the ordinary OwnerFs constructor and FUSE path. ON fixes the native cache policy when constructing OwnerFs; it cannot be switched online. The controller obtains an unforgeable current Home permit and a no-follow source descriptor, prepares and activates an export in its dedicated private mount thread, then uses a fixed configured runtime to create the container.

Only the trusted idle/identity helper runs before final source, namespace, unique mount and current grant validation. Workload exec goes through the controller and repeats those checks. The workload has a non-root UID/GID, no capabilities, a read-only rootfs and only its workspace/proc mounts. JSON requests accept a workspace name, not an arbitrary source path. The control directory/socket are administrator-only, with bounded requests and an operation ledger. Stop and status remain available when workload admission reaches the ledger limit.

The same controller retains the export and final namespace/root claims. Cleanup stops the container, observes its stopped state, normally unmounts the exact final workspace clone, deletes the runtime container, then normally detaches the original export. Descriptor-based final-clone inspection temporarily joins only the mount namespace on the owned thread and explicitly restores the controller namespace; it does not rely on the container's proc view seeing a host PID. No force/lazy detach or successful drain is inferred from PID absence. Command output, exits and failures stay in the administrator control directory.

## Incomplete boundary

This is **partial experimental implementation**, not accepted container functionality or a production enable switch. The original source slice lacked runc. Following explicit human authorization, fixed official runc v1.5.2 was installed on isolated afs-g2-micro and [runtime-only admission passed](evidence/20261007-runc-runtime/README.md). This does not qualify the AFS managed controller or OwnerFs semantics. Physical mount tests and source gates remain independently scoped.

A failed create with unrecognizable runtime state remains Unknown. Captured final handles are retained while the controller is alive; a failed observer is reconciled by exact physical inspection, and uncertain/wrong identity blocks cleanup success. No restart reconciliation exists: a nonempty runtime directory or existing controller artifacts refuses automatic adoption. Unexpected Node/controller death still requires reconciliation. The experiment does not implement production Agent READY or a revocation/drain ACK.

Necessary append/SEEK_CUR, classic kernel locks, mixed mmap/watch, permissions, revocation and restart gates remain G2.12 requirements before usable ON qualification. A build or independent mount test does not close them. G2.13 requires actual same-candidate OFF/ON/ext4 correctness and paired timings after the selected functional gate. No new benchmark ratio or current-candidate standard PASS is claimed here.

## Validation and next

All Rust and protocol tests run on ARM64 Linux. The maintained [source-gate driver](acceptance/source-slice-linux.py) accepts explicit source/input/target/output paths; no per-round Python snapshot is copied into evidence. Final source identity, raw commands/exits, first failures, physical mount accounting and review go in the [slice evidence](evidence/20261007-native-workspace/README.md). The original source-slice checkpoint had no managed-runtime run. Subsequent [6d managed lifecycle](evidence/20261007-managed-workspace/README.md) passed in its bounded scope, and [paired small performance data](evidence/20261007-container-perf/README.md) was collected. Mixed [append/locks/watch failures](evidence/20261007-managed-semantics/README.md) keep full G2.12/G2.13 in progress. The missing-runtime blocker is resolved; those records do not qualify production ON.

Official runtime/helper/initial rootfs identities and runtime capabilities/cleanup are frozen in the admission packet. runc leaves devices/symlinks under rootfs/dev, so use a fresh pinned regular-file rootfs for AFS trusted-tree startup; restart reconciliation remains unqualified. Before the managed product case, freeze its final fixture, capacity, existing processes/mounts and test criteria once; first qualify the single managed container's create/final-view/exec/stop/normal-detach path and relevant failure paths, then the small paired container access case. Stop affected runs and request help for actual environment blockers. Reuse unchanged functional results with their original version; retain all ordinary baseline data and FAIL without repeat tuning.

## Current independent runtime item — 2026-10-07

The user deferred failed3FS baseline qualification and returned to this lane. Reuse the closed append/classic-lock diagnostics; do not invent a userspace lseek or invalidation workaround to waive mixed semantics. The next [active source-injection rejection case](native-source-rejection-slice.md) tests an uncovered real-runtime protocol boundary with the same6d package: reject an unknown source field, preserve exact active container/grant/mount identity and runtime-command set, then tiny legitimate exec and normal stop. No64MiB/performance/unchanged standard retest; this is a G2.12 subitem, not full native readiness.

The [active source-field rejection runtime](evidence/20261007-native-source-rejection/README.md) subsequently passed this bounded6d case: exact before/after identity and runtime command hashes,20B legal exec and existing public stop/normal detach chain. No separate external stopped-clone observer or graceful draining is claimed. Full ON/mixed semantics stay unqualified; next is the independent bounded-ledger cleanup boundary.
