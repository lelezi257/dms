# Validation

All product validation runs on Linux. macOS is suitable for reading and editing docs, but not for final filesystem claims.

The authoritative delivery contract is [Acceptance](../acceptance.md): environment identity, standard POSIX suites, distributed cases, performance baselines, faults, RDMA and deployment. Existing smoke commands cover individual paths; they do not replace that contract.

## Standard Checks

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
bash tests/feature-matrix.sh
```

## Targeted Checks

- `tests/feature-matrix.sh`: feature combinations.
- `scripts/dfs/r1_e2e.py`: local DFS R=1 FUSE smoke.
- `scripts/ownerfs/accept_three_vm.py`: OwnerFs multi-node acceptance entry point.
- Rust integration tests under `tests/` for config, errors, FUSE boundaries, local SDK, Meta, OwnerFs peer protocol, local storage and VFS contracts.

## Evidence Rule

A capability claim should include command, environment, behavior checked and untested boundary. Performance reports should include workload, sample count, p50 and tail data, and matching durability/visibility conditions.
