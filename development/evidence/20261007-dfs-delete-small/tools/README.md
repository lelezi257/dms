# DFS delete tools 20261007 r1

Small tooling update for G2.25 DFS delete. Scope is limited to `source/development/acceptance/dfs_manyread_small.py` and `source/development/acceptance/test_dfs_manyread_small.py`; `owner_remote_small.py` is an unchanged fixed-SHA dependency.

CLI:
- `python3 development/acceptance/dfs_manyread_small.py delete-writer --dfs-root ABS_DFS_MOUNT --output ABS_FRESH`
- `python3 development/acceptance/dfs_manyread_small.py delete-checker --dfs-root ABS_DFS_MOUNT --manifest ABS_WRITER_SUMMARY --output ABS_FRESH --checker-id B|C`

Shape: 100 distinct 4KiB files per sample, one warmup plus five measured samples. Writer timer covers only 100 unlink syscalls. Preparation, per-file fdatasync, fresh full-content/EOF verification, directory fsync and cleanup are outside the timer. Checker performs 600 actual `lstat` calls and requires ENOENT; present files, other errno, traversal, wrong manifest shape and wrong mount all fail.

Validation: `raw/r2-linux-delete-guards.*` records Linux ARM64 root unittest execution on `afs-accept-a`: 5 affected delete guards, exit 0, OK. `raw/r2-linux-help-sha.*` records CLI help and file SHA checks. Prior 4-guard run is preserved as `raw/linux-delete-guards.*`. No service/runtime/delete workload was started here.

Last evidence update: 2026-10-07T07:57:08+08:00.

## r3 manifest binding fix (2026-10-07T08:04:57+08:00)

Current stable delete checker tooling is now bound to driver `e0875b743eea419ee27bc9df5677f0af421d70f590076d44e13b79b6d38248c8` (53872 bytes) and test `658b9fb6684e57041a65fdadffcf21911705e2bbff5be018823ca8ebc03eb5a5` (25395 bytes); the reused Owner helper remains `d901442e19740143f84c106bd31d2ed6162fe0b278267cde5460a947584aaf98`. The fix tightens `validate_delete_manifest` so a checker accepts only `role=delete-writer`, a digits-digits `run_id`, exact `delete_sample_name(run_id, index)` names, writer-root `directory` paths, and `afs-dfs` FUSE mount/root identity. This closes the false `DATA_RECORDED` path where six unrelated safe basenames could be checked as ENOENT.

Linux affected guard evidence: `raw/r3b-linux-delete-guards.*` on `afs-accept-a`, exit 0, 6 tests OK. r2 writer-tool provenance remains recoverable through `r2-narrow-diff.patch`; `raw/r2-patch-restore-proof.*` verifies fixed base `e68e99e3fc72078b1131a87788828f2447a80acc` plus that patch restores driver `d001d71c982c17c0781eaa70b5ab4f89cb6e6d6fb1efecb0b6defb46360e2875` and test `143531cab4191037ecd021de7cf0fd9359cdab810e72030c19f98e742999841a`. No DFS service or runtime measurement was run in this r3 tooling fix.

## r3b identity clarification (2026-10-07T08:07:20+08:00)

Canonical tool files are frozen at driver `e0875b743eea419ee27bc9df5677f0af421d70f590076d44e13b79b6d38248c8` (53872 bytes), test `658b9fb6684e57041a65fdadffcf21911705e2bbff5be018823ca8ebc03eb5a5` (25395 bytes), Owner helper `d901442e19740143f84c106bd31d2ed6162fe0b278267cde5460a947584aaf98`. The prior reviewed/runtime checker version was driver `380c49cb7ae2664f9c5c2988b67a3e06dd08ee3d48773ab8f4bb3706e0297a76`; the only source delta to canonical e087 is one stricter manifest-root text check rejecting `..` inside `fs.dfs_root.path`, captured in `r3a-380c-to-r3b-e087.patch`. Linux proof `raw/r3b-linux-restore-380c-proof.*` restores the 380c driver from e087 with that reverse patch. Runtime B/C checker results using 380c remain identified as 380c and are not rebound to e087.

Canonical e087 was validated with `raw/r3b-linux-delete-guards.*` (6 targeted tests OK) and offline against the real d001 writer manifest `dfs-delete-6d-20261007-r1/guest/a/results/delete-writer-r1/confirmed.json` via `raw/r3b-actual-manifest-validate.*` (6 rounds accepted). No mount, writer, checker runtime, or performance measurement was rerun for this identity/provenance clarification.

