# AFS comparator baseline builds

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
