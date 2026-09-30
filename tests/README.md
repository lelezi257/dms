# Test Reference

Run tests on Linux. Some filesystem tests require `/dev/fuse`, privileges or explicit ignored-test selection.

## Rust Tests

- `config_contract.rs`: CLI and TOML precedence, unknown fields and backend feature selection.
- `error_contract.rs`: structured AFS errors across TCP, UDS, REST and FUSE edges.
- `fuse_contract.rs`: FUSE session and backend contract checks; privileged and ignored where required.
- `local_sdk.rs`: UDS and SHM-oriented local SDK behavior.
- `meta_contract.rs`: Meta ping, OwnerRoots, DFS Meta and recovery-contract checks.
- `ownerfs_peer_contract.rs`: OwnerFs peer protocol boundaries.
- `rdma_lifecycle.rs`: explicit RDMA lifecycle checks when an RDMA environment is provided.
- `storage_localfs.rs`: local filesystem storage safety and range I/O.
- `vfs_contract.rs`: backend trait and VFS boundary checks.

## Scripts

- `tests/feature-matrix.sh`: verifies feature combinations for OwnerFs, DFS, zero-backend Meta and transport crates.
- `scripts/dfs/r1_e2e.py`: starts real Meta and DFS Node processes, mounts FUSE, writes a file, syncs it and reads it back.
- `scripts/ownerfs/accept_three_vm.py`: OwnerFs multi-node acceptance entry point.

## Scope Notes

Tests prove only the behavior they exercise. The suite does not currently prove production HA, complete POSIX coverage, full R=N replication, VerifiedCache, SeedLease, Spill or all crash-recovery scenarios.
