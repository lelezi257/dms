# AFS comparator baseline builds

## Current small namespace reference — 2026-10-07

[One patched3FS R2 small-delete run](../../evidence/20261007-threefs-delete-small/README.md) records all6samples and B/C600ENOENT each, median615.210/pooled509.565ops/s. Reuse priorDFS29.861/30.682 without rerun. A exceeds the predeclared1GiB allocated budget by4,067,328B (free2.99GB/floorPASS; logs1.57MB), and FDB actualwait-15 violates the9wait0 contract. Formal reference qualification stays failed/pending; no thresholds waived or environment repaired. All18ownedPIDs/mounts are gone, old10processes/all old mountIDs unchanged from raw; frozen TARGET-only postcheck falseFAIL preserved with supplemental ID audit and a repair-before-reuse TODO. Shared environment/cacheUNOBSERVED/512KiB vs4KiB allocation/explicitARM patch prohibit stock or full parity claims. User decision: defer baseline qualification to a separate topic and return to container-workspace core fixes.


This directory owns reproducible scripts for the first-stage AFS comparator
baselines. They prepare real MooseFS and 3FS artifacts on Linux and record
evidence under `evidence/afs-delivery/baseline-build/`.

Scope boundaries:

- Product code under `source/` is not modified by these scripts.
- The reference 3FS checkout under `../ref/3FS` is treated as read-only. The
  scripts copy it to the build VM before initializing submodules.
- MooseFS CE v4.59.2 remains blocked for strong durable-write comparison until
  a stock-version/config proof shows that application success waits for data,
  CRC and required metadata durability. These scripts build the stock artifact;
  they do not patch MooseFS or relax AFS barriers.
- Heavy compilation runs only inside the Linux `afs-build` VM. macOS is used
  only for orchestration and file transfer.

Main entry points:

- `bin/prepare_3fs_scratch.sh` copies the fixed 3FS checkout to
  `/home/lzc.guest/afs-build/baselines/3fs-src` in `afs-build` and initializes
  submodules there.
- `bin/build_moosefs.sh` runs inside `afs-build` and builds MooseFS from the
  fixed upstream commit.
- `bin/prepare_3fs_deps.sh` runs inside `afs-build` and downloads/verifies
  FoundationDB 7.3.63 ARM packages plus the libfuse 3.16.2 release asset. It
  does not start the 3FS C++ build.
- `bin/build_3fs.sh` runs inside `afs-build`, installs/verifies FoundationDB
  7.3.63 ARM packages, checks build dependencies, applies 3FS patches, and
  builds with low parallelism.

Each script writes a timestamped log and status file. A missing dependency or a
blocked durability proof is recorded as `BLOCKED`, not as a successful baseline.

## 2026-10-07 small deletion qualification boundary

[Read-only four-role survey](../../evidence/20261007-dfs-delete-small/baseline-survey/report.json) confirms retained3FS39-entry artifacts/ELF dependencies, with no live current comparator. The retained ARM64 reference includes a disclosed compatibility patch and small-fixture resource settings; it is not stock/unmodified or a frozen fair performance baseline. [Current candidate100-file deletion](../../evidence/20261007-dfs-delete-small/README.md) is separate from formal3FS comparison. Namespace unlink/rmdir qualification can be assessed independently of three-sync data/WAL/Meta durability; original strong read/write blockers stay unchanged. Fix the small comparison artifact/resource/mount/timer contract before any measurement; do not lower old guards after failure or repair an environment in this lane.
