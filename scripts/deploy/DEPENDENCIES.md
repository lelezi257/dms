# AFS Runtime Dependencies

The release package is installed on Linux guests without Cargo, Git or a Rust toolchain.

## Required

- Linux on the target architecture used by the release package.
- `bash`, `coreutils`, `tar`, `sha256sum`, `sed`, `awk`, `grep`, `curl`.
- `fuse3` runtime and `/dev/fuse` for FUSE mounts.
- `ss` from `iproute2` for port conflict checks. If `ss` is unavailable the process controller still starts, but port validation is weaker.
- `findmnt` from `util-linux` for exact AFS FUSE mount readiness checks.

## Backend Services

- `local-file` is supported for local smoke tests and non-cluster development runs.
- `etcd` uses `meta_store = "etcd"` and `etcd_endpoint`.
- Redis uses `meta_store = "redis"` and `redis_endpoint` when the installed binary includes that backend.

Backend configuration fields do not prove a backend lane is accepted. Each selected backend must pass its own restart and parity cases before being reported as usable.

## Optional

- TLS certificate files, referenced from TOML config, for authenticated Node-to-Meta and Node-to-Node traffic.
- RDMA/RXE tools and kernel support for RDMA acceptance lanes. The package does not install kernel modules.
