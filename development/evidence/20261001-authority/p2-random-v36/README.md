# DFS random differential smoke replay — v36 A, seed 1, 500 operations

## Result

PASS on the fresh v36 replay fixture `std04-dfs-random-500-v36-timeout180-20260930T175041Z`.

The hardened `random_fs.py` DFS differential smoke replay completed seed 1 with 500/500 operations under the 180 second per-seed timeout, with strict product identity and fixture accounting checks passing.

This is bounded functional smoke evidence for the random differential driver. It is not a full STD-04 10,000-operation qualification and is not performance evidence.

## Runtime identity

- Host: `lima-afs-accept-a`
- Memory lane: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v36`
- DFS mount: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v36/mount-dfs`
- Node PID: `512383`
- Node binary: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v36/bin/afs-node`
- Node SHA256: `646a67667d434bbbe771e183558a0ae894976eaeb6e0c5b0e2d28864b61cbd99`
- Meta PID: `512353`
- Meta binary: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v36/bin/afs-meta`
- Meta SHA256: `06fb8d24b9ad02aebbeb1e3aeb5f7daa7dc516deef27a68dda2bfd2d5168d150`
- Hypothesis: `6.168.3` from `/mnt/lima-afsadata/afs-delivery/evidence/p2-random-v27/venv`

The successful run recorded these checks as PASS: Linux runtime, observed DFS backend, same-fixture filesystem, product process identity, seed execution, and differential results.

## Successful command

Run from guest path `/mnt/lima-afsadata/afs-delivery/evidence/p2-random-v36/acceptance-copy`:

```bash
sudo /mnt/lima-afsadata/afs-delivery/evidence/p2-random-v27/venv/bin/python \
  experiments/afs-acceptance/drivers/random_fs.py \
  --profile smoke --operations 500 --seeds 1 \
  --per-seed-timeout-seconds 180 \
  --backend dfs --meta memory \
  --mount /mnt/lima-afsadata/afs-delivery/p2-memory-lane-v36/mount-dfs \
  --base-dir std04-dfs-random-500-v36-timeout180-20260930T175041Z \
  --reference-dir /mnt/lima-afsadata/afs-delivery/evidence/p2-random-v36/ref-ext4-500-timeout180-20260930T175041Z \
  --run-dir /mnt/lima-afsadata/afs-delivery/evidence/p2-random-v36/run-dfs-500-timeout180-20260930T175041Z \
  --process-pid 512383 --meta-process-pid 512353
```

## Evidence

Successful run:

- Raw driver log and top-level JSON: `run-dfs-500-timeout180-20260930T175041Z/random-dfs-500-timeout180.log`
- Identity proof: `run-dfs-500-timeout180-20260930T175041Z/artifacts/std-04-random/identity.json`
- Per-seed result: `run-dfs-500-timeout180-20260930T175041Z/artifacts/std-04-random/seed-results.json`
- Operation trace: `run-dfs-500-timeout180-20260930T175041Z/artifacts/std-04-random/seeds/seed-1/trace.json`
- Shrink corpus: `run-dfs-500-timeout180-20260930T175041Z/artifacts/std-04-random/seeds/seed-1/shrink-corpus.json`
- Acceptance driver copy: `acceptance-copy/`

Preserved preflight attempt:

- `run-dfs-500-timeout180-20260930T175017Z/random-dfs-500-timeout180.log`
- Status: `BLOCKED` before seed execution because the target fixture base under the DFS mount did not exist for the driver's scope gate.

## Accounting

Successful run:

- Profile: `smoke`
- Selected seeds: `[1]`
- Operations requested per seed: `500`
- Operations executed: `500`
- Timeout: `180s`
- Seed status: `PASS`
- Result counts: `PASS=1`, `FAIL=0`, `INCONCLUSIVE=0`, `BLOCKED=0`, `NOT_RUN=0`
- Trace SHA256: `4c1f94d96a3b902babd57f1100ab15a659d2583d8302f413b13cbcc2c0499ee7`
