# Container workspace source slice

2026-10-07; baseline main `0891cbfe558cdda7c8d780b7fa9e8f97329e2554`, mechanism reference [Issue42](https://github.com/lelezi257/dms/issues/42) / [PR43](https://github.com/lelezi257/dms/pull/43) head `80b0bca3d9d86a1357aa745bb65abfb567f5623f`.

**Decision:** pjdfstest qualifies functional completeness. The first performance lane is container-mounted workspace access. Ordinary FUSE/local/remote/DFS timings and failures remain available for later focused optimization. This slice does not reopen historical G1 8/8 or change previous acceptance conclusions.

## Implemented boundary

An explicitly enabled, administrator-only experiment connects Node to one managed Home workspace container. Default OFF uses the ordinary OwnerFs constructor and FUSE path. ON fixes the native cache policy when constructing OwnerFs; it cannot be switched online. The controller obtains an unforgeable current Home permit and a no-follow source descriptor, prepares and activates an export in its dedicated private mount thread, then uses a fixed configured runtime to create the container.

Only the trusted idle/identity helper runs before final source, namespace, unique mount and current grant validation. Workload exec goes through the controller and repeats those checks. The workload has a non-root UID/GID, no capabilities, a read-only rootfs and only its workspace/proc mounts. JSON requests accept a workspace name, not an arbitrary source path. The control directory/socket are administrator-only, with bounded requests and an operation ledger. Stop and status remain available when workload admission reaches the ledger limit.

The same controller retains the export and final namespace/root claims. Cleanup stops the container, observes its stopped state, normally unmounts the exact final workspace clone, deletes the runtime container, then normally detaches the original export. Descriptor-based final-clone inspection temporarily joins only the mount namespace on the owned thread and explicitly restores the controller namespace; it does not rely on the container's proc view seeing a host PID. No force/lazy detach or successful drain is inferred from PID absence. Command output, exits and failures stay in the administrator control directory.

## Incomplete boundary

This is **partial experimental implementation**, not accepted container functionality or a production enable switch. Actual runtime admission is BLOCKED: the available Linux environments lack runc. No runtime was installed or repaired during this slice; the environment choice has been requested. Physical mount tests and source gates can run independently.

A failed create with unrecognizable runtime state remains Unknown. Captured final handles are retained while the controller is alive; a failed observer is reconciled by exact physical inspection, and uncertain/wrong identity blocks cleanup success. No restart reconciliation exists: a nonempty runtime directory or existing controller artifacts refuses automatic adoption. Unexpected Node/controller death still requires reconciliation. The experiment does not implement production Agent READY or a revocation/drain ACK.

Necessary append/SEEK_CUR, classic kernel locks, mixed mmap/watch, permissions, revocation and restart gates remain G2.12 requirements before usable ON qualification. A build or independent mount test does not close them. G2.13 requires actual same-candidate OFF/ON/ext4 correctness and paired timings after the selected functional gate. No new benchmark ratio or current-candidate standard PASS is claimed here.

## Validation and next

All Rust and protocol tests run on ARM64 Linux. The maintained [source-gate driver](acceptance/source-slice-linux.py) accepts explicit source/input/target/output paths; no per-round Python snapshot is copied into evidence. Final source identity, raw commands/exits, first failures, physical mount accounting and review go in the [slice evidence](evidence/20261007-native-workspace/README.md). Current actual container lifecycle and performance remain NOT_RUN/BLOCKED; G2.12/G2.13 stay in progress.

After runtime environment admission, freeze runtime/helper/rootfs identities, capabilities, capacity, existing processes/mounts and test criteria once; first qualify the single managed container's create/final-view/exec/stop/normal-detach path and relevant failure paths, then the small paired container access case. Stop affected runs and request help for actual environment blockers. Reuse unchanged functional results with their original version; retain all ordinary baseline data and FAIL without repeat tuning.
