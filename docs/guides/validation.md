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

## Posted RDMA Cancellation

The Linux diagnostic runner [check-rdma-cancellation.py](../../scripts/check-rdma-cancellation.py)
requires GDB, root-visible `rdma` resource inventories, a working RXE device and
an unstripped native-debug test binary. Build it with
`cargo test --all-features --test rdma_lifecycle --no-run`, then run:

```sh
sudo python3 scripts/check-rdma-cancellation.py \
  --binary /absolute/path/to/rdma_lifecycle-test-binary \
  --source "$PWD" --device rxe0 \
  --evidence /absolute/path/to/new-evidence-directory
```

It pauses a real posted data WQE before CQ consumption, cancels the caller,
checks retained resources, resumes the worker and checks normal release before
process exit. It uses diagnostic NodeData; it does not qualify OwnerFs/DFS fault
matrices or prove physical DMA is still pending. See the [scoped evidence](../../development/evidence/20261001-posted-rdma-cancellation/README.md).
