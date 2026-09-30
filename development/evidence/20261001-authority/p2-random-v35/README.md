# DFS random differential smoke replay — v35 A, seed 1, 500 operations

## Result

PASS. The hardened `random_fs.py` DFS differential smoke replay completed seed 1 with 500/500 operations under the 180 second per-seed timeout.

This is bounded functional smoke evidence for the random differential driver. It is not a full STD-04 10,000-operation qualification and is not performance evidence.

## Runtime identity

- Host: `lima-afs-accept-a`
- Memory lane: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v35`
- DFS mount: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v35/mount-dfs`
- Node PID: `511382`
- Node binary: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v35/bin/afs-node`
- Node SHA256: `899e820c5fae43c814be5a0d349efaca4895fff772b2a8c9412655556974112f`
- Meta PID: `511352`
- Meta binary: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v35/bin/afs-meta`
- Meta SHA256: `4745a43393fabd0ac4507eb6fd9a7fff9ff8be236f4ed968c5ed16e0cb75ef42`
- Hypothesis: `6.168.3` from `/mnt/lima-afsadata/afs-delivery/evidence/p2-random-v27/venv`

The identity gate recorded product process identity, Linux runtime, observed DFS backend, and same-fixture filesystem checks as PASS.

## Command

Run from guest path `/mnt/lima-afsadata/afs-delivery/evidence/p2-random-v35/acceptance-copy`:

```bash
sudo /mnt/lima-afsadata/afs-delivery/evidence/p2-random-v27/venv/bin/python \
  experiments/afs-acceptance/drivers/random_fs.py \
  --profile smoke --operations 500 --seeds 1 \
  --per-seed-timeout-seconds 180 \
  --backend dfs --meta memory \
  --mount /mnt/lima-afsadata/afs-delivery/p2-memory-lane-v35/mount-dfs \
  --base-dir std04-dfs-random-500-v35-timeout180-20260930T173449Z \
  --reference-dir /mnt/lima-afsadata/afs-delivery/evidence/p2-random-v35/ref-ext4-500-timeout180-20260930T173449Z \
  --run-dir /mnt/lima-afsadata/afs-delivery/evidence/p2-random-v35/run-dfs-500-timeout180-20260930T173449Z \
  --process-pid 511382 --meta-process-pid 511352
```

## Evidence

- Raw driver log and top-level JSON: `run-dfs-500-timeout180-20260930T173449Z/random-dfs-500-timeout180.log`
- Identity proof: `run-dfs-500-timeout180-20260930T173449Z/artifacts/std-04-random/identity.json`
- Per-seed result: `run-dfs-500-timeout180-20260930T173449Z/artifacts/std-04-random/seed-results.json`
- Operation trace: `run-dfs-500-timeout180-20260930T173449Z/artifacts/std-04-random/seeds/seed-1/trace.json`
- Shrink corpus: `run-dfs-500-timeout180-20260930T173449Z/artifacts/std-04-random/seeds/seed-1/shrink-corpus.json`
- Acceptance driver copy: `acceptance-copy/`

## Accounting

- Profile: `smoke`
- Selected seeds: `[1]`
- Operations requested per seed: `500`
- Operations executed: `500`
- Timeout: `180s`
- Seed status: `PASS`
- Result counts: `PASS=1`, `FAIL=0`, `INCONCLUSIVE=0`, `BLOCKED=0`, `NOT_RUN=0`
- Trace SHA256: `4c1f94d96a3b902babd57f1100ab15a659d2583d8302f413b13cbcc2c0499ee7`
