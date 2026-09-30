# OwnerFs v36 upstream pjdfstest replay

## Result

FAIL on the full pinned pjdfstest run against actual v36 OwnerFs A.

The strict `standard.py` driver ran the complete pinned upstream pjdfstest suite with no exclusions. It discovered and executed all 236 test files, accounted all 8819 TAP checks, and preserved raw TAP output. The run did not time out.

## Target identity

- Host: `lima-afs-accept-a`
- Published source/evidence commit from root: `9933048b041d6ac24dae0c23f86645e710f9aa81`
- Memory lane: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v36`
- OwnerFs mount: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v36/mount-ownerfs`
- Owner workspace: `owner-memory-lane-20260930T174838Z`
- Node PID: `512383`
- Node SHA256: `646a67667d434bbbe771e183558a0ae894976eaeb6e0c5b0e2d28864b61cbd99`
- Meta PID: `512353`
- Meta SHA256: `06fb8d24b9ad02aebbeb1e3aeb5f7daa7dc516deef27a68dda2bfd2d5168d150`
- Backend: `ownerfs`
- Meta backend: `memory`

## Suite and driver identity

- pjdfstest suite root: `/mnt/lima-afsadata/afs-acceptance/suites-reference/src/pjdfstest`
- pjdfstest HEAD: `d25636a227606f8960e5179741d8f4ad7030ef41`
- pjdfstest executable SHA256: `83f27ae21a4de5c2dabc447238c83f21c1a17558e61762ec52fe7ffbcf61f780`
- Driver copy: `driver-current/experiments/afs-acceptance/drivers/standard.py`
- Input manifest: `input-manifest.json`

## Successful execution command

Run from guest path `/mnt/lima-afsadata/afs-delivery/evidence/p2-posix-owner-v36/driver-current`:

```bash
sudo python3 experiments/afs-acceptance/drivers/standard.py \
  --profile full \
  --timeout 1800 \
  --suite-root /mnt/lima-afsadata/afs-acceptance/suites-reference/src/pjdfstest \
  --backend ownerfs \
  --meta memory \
  --mount /mnt/lima-afsadata/afs-delivery/p2-memory-lane-v36/mount-ownerfs \
  --base-dir owner-memory-lane-20260930T174838Z/std01-ownerfs-full-v36-20260930T175904Z \
  --run-dir /mnt/lima-afsadata/afs-delivery/evidence/p2-posix-owner-v36/full-236-ownerfs-standard-20260930T175904Z \
  --process-pid 512383 \
  --meta-process-pid 512353
```

## Accounting

- Discovered files: `236`
- Executed files: `236`
- TAP checks observed/accounted: `8819/8819`
- TAP ok: `8800`
- TAP not ok: `19`
- Unexpected failures: `10`
- TODO count: `28`
- TODO not-ok count: `9`
- SKIP count: `0`
- Return code: `1`
- Duration: `156.577s`
- Timed out: `false`
- Fixture kept for failure: `/mnt/lima-afsadata/afs-delivery/p2-memory-lane-v36/mount-ownerfs/owner-memory-lane-20260930T174838Z/std01-ownerfs-full-v36-20260930T175904Z/.afs-std01-pjdfstest-20260930T175910Z-39454b69`

## Unexpected failures

All 10 unexpected failures came from `rename/23.t`, which checks that `rename(src, dst)` succeeds when `dst` is multiply linked and that the remaining hardlink to the replaced destination remains valid with link count 1.

Observed failure pattern across regular, FIFO, block, char, and socket cases: after the rename, `lstat` on the remaining hardlink returned `ESTALE` instead of the expected object type and `nlink=1`. The paired `test_check` lines also failed.

Failure details are stored in `unexpected-failures.json` and in the raw TAP file.

## Preserved preflight attempt

`full-236-ownerfs-standard-20260930T175842Z` is preserved as `BLOCKED` before execution. It used the driver's default suite path, which was absent on A, so pinned suite identity and discovery failed. Product identity checks in that preflight passed. The successful execution attempt above used the located pinned suite path explicitly.

## Remote B note

B v36 OwnerFs was not run. The existing strict driver verifies Node and Meta process identity through local `/proc/<pid>`. B has Node PID `8313` with the expected Node SHA, but the v36 Meta PID `512353` is on A, so the current driver cannot safely prove B Node plus A Meta identity from a single B-local run without weakening the guard.

## Evidence files

- Proof JSON: `full-236-ownerfs-standard-20260930T175904Z/artifacts/std-01-pjdfstest/proof.json`
- Identity JSON: `full-236-ownerfs-standard-20260930T175904Z/artifacts/std-01-pjdfstest/identity.json`
- TAP accounting: `full-236-ownerfs-standard-20260930T175904Z/artifacts/std-01-pjdfstest/tap-accounting.json`
- Raw TAP: `full-236-ownerfs-standard-20260930T175904Z/artifacts/std-01-pjdfstest/pjdfstest.stdout.tap`
- Raw stderr: `full-236-ownerfs-standard-20260930T175904Z/artifacts/std-01-pjdfstest/pjdfstest.stderr.log`
- Command JSON: `full-236-ownerfs-standard-20260930T175904Z/artifacts/std-01-pjdfstest/command.json`
