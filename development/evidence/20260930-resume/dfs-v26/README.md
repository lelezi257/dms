# DFS v26 upstream POSIX replay

Date: 2026-10-01.

Scope: evidence-only full STD-01 pjdfstest replay on `afs-accept-a` against the deployed memory-backed DFS v26 lane. No product source edits, Cargo commands, service restarts, or exclusions were used in this lane.

## Target identity

- VM: `lima-afs-accept-a`, Linux `6.8.0-142-generic`, `aarch64`.
- Lane: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v26`.
- DFS mount: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v26/mount-dfs`, `source=afs-dfs`, `fstype=fuse`.
- Meta PID: `442414`; binary SHA256 `9e20ef1c15b8dce0e19e5d08de11460b3b026f5b2ea418a1eb71daa7c55d2846`.
- Node PID: `442444`; binary SHA256 `bd424647d5d937979f3e2832564e276a1e114fd7cda7c063b7371a72cfc64c1d`.
- pjdfstest suite HEAD: `d25636a227606f8960e5179741d8f4ad7030ef41`; executable SHA256 `83f27ae21a4de5c2dabc447238c83f21c1a17558e61762ec52fe7ffbcf61f780`.

## Full strict standard driver run

Path: `full-236-dfs-standard-current/artifacts/std-01-pjdfstest/`.

Command shape: current `experiments/afs-acceptance/drivers/standard.py --profile full --timeout 1800`, run as root with `--backend dfs --meta memory`, Node PID `442444`, Meta PID `442414`, and base directory under `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v26/mount-dfs/std01-dfs-full-v26`.

Result: PASS.

- Files: 236 discovered / 236 executed.
- TAP checks: 8819 observed and accounted.
- TAP ok: 8810.
- TAP not ok: 9, all upstream TODO.
- TODO count: 28.
- Unexpected failures: 0.
- Skips: 0.
- Return code: 0.
- Timeout: 1800 seconds, not reached.
- Duration: 1218.784 seconds.
- Fixture cleanup: PASS.

Strict proof checks passed: pinned suite identity, root harness, mount identity, base directory scope, backend selection, Linux runtime, observed target backend, same fixture filesystem, product process identity, discovery, subprocess bound, TAP accounting, UID-drop coverage, pjdfstest result and cleanup fixture.

This is DFS STD-01 memory-backend development evidence only. It does not declare the broader release gate complete.
