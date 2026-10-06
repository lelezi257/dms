# AFS Runtime Dependencies

The release package is installed on Linux guests without Cargo, Git or a Rust toolchain.

## Required

- Linux on the target architecture used by the release package.
  The G1 trial is qualified on Ubuntu 24.04 ARM64 / Linux 6.8 with ext4.
  Remote OwnerFs shared mmap needs the kernel-advertised
  `FUSE_DIRECT_IO_ALLOW_MMAP` capability; older kernels are not qualified.
- `bash`, `coreutils`, `tar`, `sha256sum`, `sed`, `awk`, `grep`, `curl`,
  `python3`.
- GNU `timeout` (from `coreutils`) bounds mount inspection and unmount commands.
  `afs-selfcheck` also wraps its own Python probe in GNU `timeout` so blocked
  filesystem I/O has an outer wall-clock limit.
- `fuse3` runtime and `/dev/fuse` for FUSE mounts.
- `ss` from `iproute2` for port conflict checks. If `ss` is unavailable the process controller still starts, but port validation is weaker.
- `findmnt` from `util-linux` for exact AFS FUSE mount readiness checks and
  `afs-selfcheck` mount validation.
- `openssl` when using `afs-trial-config` to generate single-node or two-node
  trial TLS material.
- Shared libraries reported by current Linux ARM64 package `ldd.txt` include
  `libibverbs.so.1`, `libnl-route-3.so.200`, `libnl-3.so.200`, `libgcc_s.so.1`,
  `libm.so.6`, `libc.so.6` and the target dynamic loader. Install the matching
  distro packages before running the binaries. FUSE remains a runtime
  requirement through `fuse3` and `/dev/fuse` even when `libfuse3` does not
  appear in `ldd.txt`.

## Backend Services

- `memory` is supported for disposable demos and loses namespace state whenever
  `afs-meta` exits.
- `local-file` is supported for persistent trial runs, restart checks and local
  smoke tests.
- `etcd` uses `meta_store = "etcd"` and `etcd_endpoint`.
- Redis uses `meta_store = "redis"` and `redis_endpoint` when the installed binary includes that backend.

Backend configuration fields do not prove a backend lane is accepted. Each selected backend must pass its own restart and parity cases before being reported as usable.

## Optional

- TLS certificate files, referenced from TOML config, for authenticated Node-to-Meta and Node-to-Node traffic.
- RDMA/RXE tools and kernel support for RDMA acceptance lanes. The package does not install kernel modules.
