# p2 random v30 DFS smoke evidence

Scope: bounded hardened random driver DFS seed 1, 500 operation smoke against live v30 A memory lane. This is not a full STD-04 release run or performance qualification.

## Identity

- Mount: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v30/mount-dfs` (`afs-dfs`).
- Node PID/SHA: `509042` / `e4fd549487967518fb982963f3291096327d5f38c16a59cac0f5561ba07e442a`.
- Meta PID/SHA: `509012` / `ab7ce32ed9e0f212f4aa8341c0ca5ff32fd603c1a598d5e0f8e901db44dbb91d`.
- Python: reused v27 evidence venv `/mnt/lima-afsadata/afs-delivery/evidence/p2-random-v27/venv/bin/python`, Hypothesis `6.168.3`.
- Guard checks on timeout180 run: `{'linux-runtime': True, 'observed-target-backend': True, 'product-process-identity': True, 'same-fixture-filesystem': True}`.

## Original 60s run retained

- Run directory: `run-dfs-500-20260930T165748Z/`.
- Status: `INCONCLUSIVE`; reason `seed execution timed out or cleanup could not prove a clean result`.
- Operations executed: `327` of `500`; timeout `True` at `60.0` seconds.
- Mismatch: `None`; trace SHA256 `d1bdda562f6b0f9d31aac324c0d0797854ca68ab1d005a48163ca6befed0506b`.

## Timeout180 functional rerun

- Run directory: `run-dfs-500-timeout180-20260930T165944Z/`.
- Target fixture base: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v30/mount-dfs/std04-dfs-random-500-v30-timeout180-20260930T165944Z`.
- Reference fixture base: `/mnt/lima-afsadata/afs-delivery/evidence/p2-random-v30/ref-ext4-500-timeout180-20260930T165944Z`.
- Status: `PASS`.
- Counts: `{'BLOCKED': 0, 'FAIL': 0, 'INCONCLUSIVE': 0, 'NOT_RUN': 0, 'PASS': 1}`.
- Operations executed: `500` of `500`; timeout `False` at `180.0` seconds.
- Trace SHA256: `4c1f94d96a3b902babd57f1100ab15a659d2583d8302f413b13cbcc2c0499ee7`.

## Raw files

- `run-dfs-500-20260930T165748Z/random-dfs-500.log`: original 60s inconclusive command header and JSON.
- `run-dfs-500-timeout180-20260930T165944Z/random-dfs-500-timeout180.log`: timeout180 command header and JSON.
- `*/artifacts/std-04-random/identity.json`: exact mount/process identity.
- `*/artifacts/std-04-random/seed-results.json`: seed result accounting.
- `*/artifacts/std-04-random/seeds/seed-1/trace.json`: raw operation trace.
