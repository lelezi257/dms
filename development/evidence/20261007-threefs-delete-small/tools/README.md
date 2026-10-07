# 3FS delete small tool evidence r1

Updated: 2026-10-07T08:37:56+08:00

This packet records the bounded first-party 3FS namespace delete adapter for `development/threefs-delete-small-slice.md`. It adds only `threefs_delete_small.py` and `test_threefs_delete_small.py`; Owner delete primitives are reused by fixed SHA `d901442e19740143f84c106bd31d2ed6162fe0b278267cde5460a947584aaf98`.

Current stable tool identity:

- driver `06d6c4321ac23ef258d4d8cac242bae6a6b32a0f9658cb4e420e512dc32783bd` (21444 bytes)
- tests `4ebd0f4fe9aa9764416227038461af3740d118296d2370edf2ee53ac35000f31` (10247 bytes)
- Owner helper `d901442e19740143f84c106bd31d2ed6162fe0b278267cde5460a947584aaf98` (20289 bytes)

CLI for entry/runtime:

```
python3 threefs_delete_small.py writer --threefs-root ABS_HF3FS_FIXTURE_MOUNT/test --output ABS_FRESH
python3 threefs_delete_small.py checker --threefs-root ABS_HF3FS_FIXTURE_MOUNT/test --manifest ABS_WRITER_CONFIRMED --output ABS_FRESH --checker-id B|C
```

The tool accepts an hf3fs root only when `findmnt -T` reports source `hf3fs.afs_3fs_delete_v84_20261007_r1`, fstype `fuse.hf3fs`, a mount target ending `/afs-delivery/threefs-delete-v84-20261007-r1/mount`, and the supplied root is exactly mount-relative `test`. Other existing directories under the same mount are rejected before ENOENT checks.

Linux guards: `raw/r3-linux-guards.*` on `afs-accept-a`, exit 0, 7 tests OK. Superseded root-unbound version `7d1d75c.../d73051...` is recorded as unsafe for runtime because it could check the wrong empty subdirectory; `raw/r2-root-unbound-restore-proof.*` and `r2-root-binding-fix-*.patch` preserve that provenance without copying full source.

No service, mount workload, writer/checker runtime, or performance measurement was run by this agent.
