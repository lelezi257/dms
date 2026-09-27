# OwnerFs implementation acceptance evidence

Date: 2026-09-26

## Scope

This review covers only the new acceptance tooling for the AFS OwnerFs line:

- `tests/ownerfs_acceptance.py` — real-process, three-Linux-node functional acceptance runner for `afs-meta` + two `afs-node` instances in P2P/OwnerFs mode.
- `scripts/ownerfs/accept_three_vm.py` — compatibility entrypoint that runs the acceptance runner from `scripts/ownerfs`.
- `scripts/ownerfs/s5_posix_workload.py` — copied S5 POSIX workload contract used by the old HomeFs W1 bench.
- `scripts/ownerfs/w1_diagnostics.py` — strict OwnerFs W1 value runner using the copied S5 workload contract for OwnerFs, Native FS, thin FUSE, and MooseFS paths.
- `scripts/ownerfs/s5_distributed_workload.py` — copied S5 cross-node W2 stage workload from the old HomeFs bench.
- `scripts/ownerfs/w2_segment_diagnostics.py` — OwnerFs W2 segmented diagnostic runner for prepared A/B OwnerFs workspaces and MooseFS mounts.

No production Rust, roadmap/status docs, push, merge, or release action was performed.

## Functional acceptance contract encoded

`tests/ownerfs_acceptance.py` encodes the same acceptance shape as the S5 HomeFs three-VM script, adapted to the AFS binaries and `/ownerfs` namespace:

1. Linux, binary, FUSE, and etcd-intent preflight.
2. Start persistent `afs-meta` on C and `afs-node` on A/B with `--fs ownerfs`, gRPC data mode, and real FUSE mounts.
3. Verify `/ownerfs` namespace exists on both nodes.
4. Create an OwnerFs root on A and require a management location API response.
5. Verify A close-to-open then B reopen returns exact bytes.
6. Verify A rewrite then B reopen sees new bytes.
7. Verify rename, unlink, and rmdir visibility across nodes.
8. Verify concurrent root creation yields one authoritative location.
9. Verify old remote file descriptors remain bound after rename/name reuse and unlink/name reuse.
10. Verify alternating cross-node file lengths are exact.
11. Verify Meta restart preserves root ownership and B read.
12. Verify Home/P2P service restart returns a bounded failure while down and recovers after restart.
13. Verify cross-root rename returns `EXDEV` through `os.rename`.

The runner prints one machine-readable JSON object plus `PASS` or `FAIL`, uses a run-specific automatic port range unless explicit ports are supplied, passes the current `etcd_endpoint`, `advertise_endpoint`, config, and TLS CLI fields through to `afs-meta`/`afs-node`, verifies root location through `OwnerRoots.LookupRoot` via `grpcurl` by default, preserves bounded log tails in the JSON, and cleans up processes and FUSE mounts with lazy unmount fallback. Cleanup is best-effort: unmount/kill/temp-dir failures and timeouts are recorded in `cleanup_errors` and must not hide the original failed step.

## W1 value contract encoded

`scripts/ownerfs/w1_diagnostics.py` now follows the old S5/HomeFs W1 contract instead of the temporary smoke workload. It imports the copied `scripts/ownerfs/s5_posix_workload.py`, requires all four backend roots, performs one warmup per backend, runs two independent sessions, and records six valid same-scene rounds per session. Backend order rotates each round and reverses after the first full cycle, matching the old bench shape.

The value gate is per-session `p50(ownerfs batch_wall_us) / p50(moosefs batch_wall_us) <= 0.80`. OwnerFs must be a fixed, pre-created workspace path under the namespace: `<afs-fuse-mount>/ownerfs/<workspace>`. The runner validates it with `findmnt -T <path>`, rejects native paths, direct mountpoints, and the namespace directory `<mount>/ownerfs` itself, and can attach a `LookupRoot` JSON receipt with `--ownerfs-location-receipt` plus `--ownerfs-expected-home-node` to prove the workspace is owned by A before timing begins. W1 then creates per-run directories only inside that workspace, so Meta reserve/catalog for workspace creation is not part of the measured main path. MooseFS and thin FUSE must still be direct mountpoints. Native FS must be a distinct writable root, with an optional `--require-native-mount` for environments that want native mounted explicitly. The output includes raw samples, p50/p95/p99, hashes for runner/workload/OwnerFs binary, and a receipt. Missing paths, duplicate backend roots, partial runs, or mount validation failures cannot produce PASS or valid ratios.


## W2 segmented diagnostic contract encoded

`scripts/ownerfs/w2_segment_diagnostics.py` reuses the old HomeFs S5 W2 shape without declaring a W2 value gate. It expects an already-running three-VM lab with A/B OwnerFs mounts and A/B MooseFS mounts, validates OwnerFs roots as fixed pre-created workspaces `<afs-fuse-mount>/ownerfs/<workspace>`, stages the copied `s5_distributed_workload.py` and `s5_posix_workload.py` on A/B, then runs the old sequence for each backend: A prepare, B first read, B repeat read, B overwrite, A read updated, and A cleanup.

Each round alternates OwnerFs/MooseFS order, uses the same seed for both backends, records raw per-file samples for the four measured stages, records each stage ledger, and reports p50/p95/p99 total wall time plus OwnerFs/MooseFS diagnostic ratios. `PASS` only means all requested W2 stages completed with correctness checks; it is not a performance acceptance threshold.

## Current implementation blockers observed from code/readiness

The harness is ready to expose current OwnerFs gaps rather than mask them:

- `src/node.rs` now creates a production `OwnerFs::new_local(...)` instance and requires `meta_endpoint`, `advertise_endpoint` when listening on `0.0.0.0`, and an etcd-backed Meta. The harness was updated to pass those fields through. This still needs a real Linux run before it can be counted as accepted.
- `src/meta/rpc.rs` now contains real `OwnerRoots` authority methods over the configured `MetaStore`; Meta REST still exposes health/ping/metrics only. The runner therefore verifies root location through gRPC `OwnerRoots.LookupRoot` with `grpcurl` by default.
- `src/node/rpc/data/owner.rs` still constructs `make_owner_files_server()` with the default service in `src/node.rs`; without an injected `OwnerFilesHandler` and peer authenticator, remote B P2P file operations should fail closed rather than pass.
- `docs/plans/2026-09-26-ownerfs-readiness.md` says real remote file operations and performance evidence still require Linux three-VM and W1/W2 revalidation. Old S5 data is not reused here.

These are product implementation gaps or environment gaps, not harness passes. The acceptance runner should remain failing until the actual semantics are implemented and exercised.

## Commands

Run functional acceptance from Linux node A after building/copying identical binaries to all hosts:

```bash
cd /workspace/dms/source
python3 tests/ownerfs_acceptance.py \
  --host-a 192.168.104.12 \
  --host-b 192.168.104.13 \
  --host-c 192.168.104.11 \
  --ssh-user lzc \
  --bin-dir /home/lzc.guest/dms-target/release \
  --etcd-endpoint http://192.168.104.11:2379 \
  --node-a-advertise-endpoint http://192.168.104.12:17410 \
  --node-b-advertise-endpoint http://192.168.104.13:17420 \
  --output /tmp/ownerfs-acceptance.json
```

Run W1 diagnostics after preparing the relevant paths:

```bash
cd /workspace/dms/source
python3 scripts/ownerfs/w1_diagnostics.py \
  --ownerfs /mnt/afs/ownerfs/job-42 \
  --ownerfs-workspace job-42 \
  --ownerfs-location-receipt /tmp/ownerfs-job-42-location.json \
  --ownerfs-expected-home-node node-a \
  --moosefs /mnt/moosefs-w1 \
  --thin-fuse /mnt/thin-fuse-w1 \
  --native /mnt/native-w1 \
  --ownerfs-binary /home/lzc.guest/dms-target/release/afs-node \
  --output /tmp/ownerfs-w1-diagnostics.json
```

Run W2 segmented diagnostics after preparing the same workspace on A/B and MooseFS on A/B:

```bash
cd /workspace/dms/source
python3 scripts/ownerfs/w2_segment_diagnostics.py   --host-a localhost   --host-b 192.168.104.13   --ssh-user lzc   --ownerfs-a /mnt/afs/ownerfs/job-42   --ownerfs-b /mnt/afs/ownerfs/job-42   --ownerfs-workspace job-42   --moosefs-a /mnt/moosefs-w2   --moosefs-b /mnt/moosefs-w2   --ownerfs-binary /home/lzc.guest/dms-target/release/afs-node   --output /tmp/ownerfs-w2-segments
```

## Verification performed

On Linux VM `dms-dev`:

```bash
cd /workspace/dms/source
python3 -m py_compile tests/ownerfs_acceptance.py scripts/ownerfs/accept_three_vm.py scripts/ownerfs/s5_posix_workload.py scripts/ownerfs/s5_distributed_workload.py scripts/ownerfs/w1_diagnostics.py scripts/ownerfs/w2_segment_diagnostics.py
mkdir -p /tmp/ownerfs-w1-neg/{mnt/ownerfs/job-42,moose,thin,native}
scripts/ownerfs/w1_diagnostics.py --ownerfs /tmp/ownerfs-w1-neg/mnt/ownerfs --moosefs /tmp/ownerfs-w1-neg/moose --thin-fuse /tmp/ownerfs-w1-neg/thin --native /tmp/ownerfs-w1-neg/native --ownerfs-binary /bin/true --output /tmp/ownerfs-w1-neg/out-namespace
scripts/ownerfs/w1_diagnostics.py --ownerfs /tmp/ownerfs-w1-neg/mnt/ownerfs/job-42 --ownerfs-workspace job-42 --moosefs /tmp/ownerfs-w1-neg/moose --thin-fuse /tmp/ownerfs-w1-neg/thin --native /tmp/ownerfs-w1-neg/native --ownerfs-binary /bin/true --output /tmp/ownerfs-w1-neg/out-native
tests/ownerfs_acceptance.py --host-b localhost --host-c localhost --bin-dir /definitely-missing-ownerfs-bin --etcd-endpoint http://127.0.0.1:2379 --preflight-only
```

Result: W1 negative cases returned structured JSON `ERROR` for both `<path>/ownerfs` namespace input and a fake native `<path>/ownerfs/job-42` workspace. This proves the runner requires a fixed pre-created workspace under a real AFS FUSE mount and refuses to measure workspace creation/catalog work as W1 main-path performance. A second negative case with duplicate backend paths returned structured JSON `ERROR` before any workload run. The acceptance preflight negative case returned structured JSON `FAIL` for a missing binary directory, proving the functional runner reports bounded failures instead of silently passing. A harness unit check also simulated a cleanup exception after a preflight failure; `execute()` still returned the original `FAIL` JSON with the cleanup problem recorded in `cleanup_errors`. Command timeout behavior was checked directly: `check=False` returns rc `124`, while `check=True` still raises `RunnerError`.

Environment note: `dms-dev` currently has a healthy single-VM apt `etcd-server` at `127.0.0.1:2379`. The `dms-s5-a/b/c` three-VM lab is now running per environment update, but this pass only performed syntax and negative-case validation; full three-VM functional acceptance and two-by-six W1 sessions still require OwnerFs production wiring, mounted four-backend paths, `grpcurl` or an equivalent gRPC client, and current binaries. No full acceptance pass is claimed here.

W2 runner syntax and negative validation were also checked on `dms-dev`: `scripts/ownerfs/w2_segment_diagnostics.py` rejects fake native `/ownerfs/<workspace>` paths with structured JSON `ERROR`, and rejects invalid `--rounds 0` before running any workload. No real W2 performance result is claimed.
