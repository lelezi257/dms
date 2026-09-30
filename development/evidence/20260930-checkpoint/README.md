# Linux development checkpoint — 2026-09-30

These are raw development results, not release acceptance. The [handoff](../../../docs/handoff.md) owns the continuation entry and [status](../../../docs/status.md) owns current capability status.

## Current candidate v25

`identity.json` identifies the stable captured 223-file snapshot; `source.sha256` lists its files. `source-comparison.json` confirms all compiled Rust/proto/native/Cargo files still match it; documentation/navigation were updated afterward. Source tar SHA256 is `09d08b944c81e6f3141efd6fa83dd046f432f8e2efab764e0403b82e2a0e1992`.

Environment: dedicated `afs-build` Ubuntu24.04.4 ARM64 Linux6.8.0-142, Rust1.95.0, jobs2/incremental0. Working source and target are on guest ext4. Product data was not tested on the host share.

| Command | Result | Raw log |
| --- | --- | --- |
| cargo fmt --all -- --check | exit0 PASS | linux-v25/fmt.log (empty successful output) |
| cargo clippy --workspace --all-targets --all-features -- -D warnings | exit0 PASS | linux-v25/clippy.log |
| cargo test -p afs --lib --all-features | exit0, 258 PASS / 2 ignored | linux-v25/lib.log |
| cargo build -p afs --all-features --bins | exit0 PASS | linux-v25/build.log; binaries.sha256 |
| cargo test -p afs --all-features --test error_contract --test meta_contract --test ownerfs_peer_contract --test rest_contract --test config_contract --test vfs_contract --test fuse_contract | exit0, 53 PASS / 5 environment ignored | linux-v25/contracts.log |
| cargo test -p afs-error | exit0, 4 PASS | linux-v25/error.log |
| built fuse_contract harness --ignored --test-threads=1, as Linux root | exit0, 5 PASS | linux-v25/fuse-real.log |
| cargo check -p afs --all-targets --no-default-features | exit0 PASS | linux-v25/check-none.log |
| preceding check + --features ownerfs | exit0 PASS | linux-v25/check-owner.log |
| preceding check + --features dfs | exit0 PASS | linux-v25/check-dfs.log |
| skill-creator quick_validate.py .codex/skills/afs-acceptance | exit0 valid | linux-v25/skill.log |
| Python unittest selected portable runner/probe modules | exit0, 77 PASS | portable-selftests.log |

The two ignored library cases require a dedicated durable Redis and explicit RXE product fixture respectively. They remain unqualified by this library run. The five normally ignored FUSE tests were explicitly rerun as root with the already-built harness. Its teardown emits fusermount `Invalid argument` after session unmount; all five assertions and process exit pass.

An initial sudo cargo invocation hit timeout124 while root rustup downloaded a separate toolchain, before any tests ran; original `fuse-root-toolchain-timeout.log` is retained. The succeeding direct-harness invocation avoids that toolchain/environment mistake; it is a separate result, not a relabel of the timeout.

Portable Python command, from a copied `<work>/experiments/afs-acceptance` Linux layout:

```sh
python3 -m unittest test_runner test_target_identity test_inventory test_deploy_driver test_standard_driver test_fsx_driver test_random_fs_driver test_locks_cross
```

This proves preparation behavior, not product case PASS. LTP driver selftests were not included in this fresh 77-test rerun; their earlier evidence remains in the original research workspace.

## Historical runtime results

- `historical-v11-pjdfstest/`: full raw TAP/discovery/accounting/command/identity for DFS PASS and OwnerFs two failures on the earlier v11 live memory lane. v25 source fixes require a new mount rerun.
- `historical-rdma/`: earlier identified A/B RXE R2 write and R1 peer read, bytes/counters/catalog/recovery proof and structured result. Not v25 qualification. Original failed fixture logs and scripts remain in the research workspace; this subset preserves the repaired proof used by the handoff.
- `significant-changes.md`: original historical implementation choices for final architecture/interface review. It is not an up-to-date test summary.

No binaries, private keys, VM disks or build caches are committed. All formal69 release cases remain NOT_RUN; environment lock is PREPARING. Full cross-mount, persistent backend, faults, performance, deployment and long-run gates remain open.

Raw evidence whitespace is preserved using `.gitattributes`; TAP alignment and empty TSV exclusion fields are not normalized.
