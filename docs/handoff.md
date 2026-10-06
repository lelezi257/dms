# AFS portable handoff — 2026-10-06

This page is refreshed under the user's explicit request to publish a coherent current code/documentation checkpoint. Start with [current checkpoint](../development/current-checkpoint.md), [three-stage checklist](../development/trial-release-goals.md), [implementation status](status.md) and [plan](../development/plan.md). They describe the same Git snapshot; use `git rev-parse HEAD` to identify a checkout.

## Where we are

G1 historical g1.5 trial is complete, 8/8. G2 is active: Owner internal optimization, DFS read-batch optimization and finite ext4 tool qualification are complete only within their bounded scopes; bind functionality/performance are in progress; current-candidate standard/installed recovery/core performance and a new performance kit are pending. G3 complex reliability and backends are deferred. Do not restart G1 or infer full POSIX/performance qualification from passing unit tests.

The current source includes all accumulated main-worktree product changes. It is later than the g1.5 binaries. [Exact input and fresh Linux validation](../development/checkpoints/20261006-current/results/README.md) accompanies this publication. Rebuild from this checkout; do not mix old ELF/package identity into its results. This publication is not a newly installed performance release.

## Trial and artifacts

[Trial guide](guides/trial.md) explains memory demonstrations and local-file-backed restartable trials. [Historical package identity](../development/current-checkpoint.md#trial-artifact) records SHA256 and g1.5's proven scope. The old archive/raw runtime data remain external artifacts and are not automatically downloaded by Git clone. The repository contains portable structured evidence and the current build/deploy scripts. Generate local TLS/configuration; do not transfer private keys, old PID files or assumed addresses.

All compilation/tests/runtime/benchmarks run on ARM64 Linux. macOS only edits, reads and orchestrates VMs. Validation commands and caveats are in [validation](../development/validation.md). A PREPARING final lock cannot produce formal performance PASS.

## Issue42 / PR43 handoff

The read handoff used PR43 head `80b0bca3d9d86a1357aa745bb65abfb567f5623f`, draft/unmerged at capture. Its experimental results and 13 raw-package checks are historical evidence, not mainline production acceptance. See [native handoff](../development/current-checkpoint.md#native-handoff) and [RFC](rfcs/0001-ownerfs-native-bind-mount.md). PR43 overlaps current main changes; reconcile contracts rather than blindly merging over recovery/permission/index fixes.

Native foundation tasks N2a, N2b1 and N2c PF1 have bounded proofs. Current production native remains disabled; the public configurable ON switch and managed lifecycle are not delivered. G2.12 must establish final Agent namespace/source/Root/epoch/Home admission, necessary append/SEEK_CUR and actual kernel lock behavior, watch/mixed mmap semantics, startup/stop and genuine cross-container reference drain before ON. Export detach alone does not stop writable container clones. G2.13 separately pairs OFF/ON/ext4 performance. OFF/FUSE work does not wait for native readiness.

## What to do next

1. Use the published checkpoint as the single starting state; check code/tool hashes before reusing validation outputs.
2. Complete Owner's current pjdfstest/fixed LTP/affected basic/local-file recovery slices, then small local read/write/delete independently.
3. Proceed to Owner remote core cases and independent native tasks. Freeze matched durability, cache/resource conditions, sample budget and noise tolerance before each comparison.
4. Enter DFS with small one-writer/many-reader first, then separate read/write/delete and larger sizes.
5. Release selected qualified core cases with an independently installed/recovery-checked package. Keep native OFF if still unqualified and report its tasks as pending.

Owner local target ≥90%ext4 throughput; remote MooseFS parity; DFS parity with 3FS under three synchronous durable copies and matched FUSE/POSIX. Do not chase small residuals indefinitely: preserve failed evidence, stop at the case budget and move to another independent item. Delete has no newly invented hard ratio.

Disk admission is per case; expand A only after preserving active services' recoverable state. No unrelated VM/service is stopped for this publication. Full large/long/complex conditions apply to dependent tasks only. The parent-FD experiment's complete 48-window/20-pair benefit FAIL is preserved; unchanged inputs are not a reason to repeat it. etcd2GiB/D17 topic, Redis last and HA remain G3. The complete G2 goal stays active.
