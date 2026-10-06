# Default-OFF container workspace source checkpoint

**PASS, bounded source scope:** main baseline `0891cbfe558cdda7c8d780b7fa9e8f97329e2554`; final 157 compiler inputs map `8ef8b7882381b80445ce13576a6b6c41b1a054fd5a101a5143d77b32c14ea267`. Actual container runtime admission is **BLOCKED** (runc absent); container lifecycle, ON semantics and performance are **NOT_RUN/unqualified**. G1 stays historical8/8 and G2.12/G2.13 remain in progress.

| Result | Version/input | Raw evidence and scope |
| --- | --- | --- |
| Control/client PASS | r3 mapf55efea6 | [Rust control](r3/native-control.log):7PASS/4ignored; [protocol](r3/control-client.log):3PASS; failure replay, conflicting request IDs, quota does not block Stop/Status, complete/fragmented/error socket responses |
| Home authorization PASS | r3 mapf55efea6 | [18PASS/1ignored](r3/native-home.log); existing permit freshness/rights/no-follow and ordinary/native constructor boundaries |
| Library/contract PASS | r3 mapf55efea6 | [562PASS/16ignored](r3/library.log); [commands](r3/source-proof.json) record config7, FUSE4/8ignored, Nodehealth20, Ownerpeer9/3ignored, VFS1; ignored tests are not passes |
| Actual native mount PASS | r3 mapf55efea6 | [4PASS/0ignored](r3/physical-native.log), explicitly root Linux: prepare/activate/normal detach, uncertain/foreign claims, distinct final clone, flags drift/wrong source rejection, restoration, clone detach while original remains intact, then original detach |
| Actual FUSE PASS | r3 mapf55efea6 | [8PASS](r3/physical-fuse.log); this is affected FUSE regression, not pjdfstest or complete POSIX |
| Feature checks PASS, warnings retained | r3 mapf55efea6 | [Owner-only](r3/owner-only.log)/[DFS-only](r3/dfs-only.log); each has2existing library dead-code warnings and2then-unfixed helper cast warnings. Do not describe them as warning-free |
| Final source gate PASS | r4 map8ef8b788 | [fmt/strict all-feature Clippy/release bins/helperTERM](r4/source-proof.json); [helper](r4/helper-idle.log) runs actual release ELF and exits0 on TERM; no actual runc container proof |

[Closure](closure.json) composes r3 unaffected checks with r4 fresh gates. The **only** r3→r4 compiler change is the helper signal handler function→pointer cast; library, Node, Home, config, tests, physical mount code, protocol tools, environment and criteria are unchanged. Their results are reused, not rerun. [r3map](inputs-r3.json)/[finalmap](inputs.json) make that claim independently checkable. r4 receipt selects only4gates and is not presented as a fresh full-suite run.

Final release ELF SHA256:

- `afs-meta`: `08aee9fec2e7e12ce03912fa85e8bded28b2162de78c2c9c36703a9a1a6e58b8`
- `afs-node`: `d6e7cd11a610e7f0839d2a3d2337cd92678432e15fcffb6fd2fa6a00860058b8`
- `afs-workspace-probe`: `c9c9fd1a3568c872fb975add7595df43f46016c674647ef48ac2103db979ddb7`

## Failures retained

[r1](r1/source-proof.json)/[inputs](inputs-r1.json): first compile failed on moved Node state, metadata method access and unused import; raw compiler log retained. [r2](r2/source-proof.json)/[inputs](inputs-r2.json): applicable checks passed, strict lint failed on unused permit name and collapsible conditional; corrected, not hidden. [r3](r3/source-proof.json): applicable checks passed, strict lint failed on helper function-to-integer casts; r4 corrects only those casts. Failed attempts are not counted as complete source passes.

The auxiliary mount agent mistakenly ran throwaway Rust checks on macOS before correction. Those results/artifacts (`/tmp/native-mount-check-20261007`) are **excluded** from this evidence. All counted Rust, filesystem and Python protocol results here are ARM64 Linux. Physical tests use explicit privileged admission and assertions; no silent skip is counted as success.

## Reproduction and limits

[Maintained driver](../../acceptance/source-slice-linux.py), [controller client](../../../scripts/ownerfs/native-workspace-control.py), [protocol regression](../../../scripts/ownerfs/test_native_workspace_control.py); [tool hashes](tools.json). Evidence retains commands/exits/input maps/raw results, without Python source snapshots, binaries or private TLS material. Early r1/r2 driver did not include the protocol-client step; recorded argv and gate accounting are authoritative for each attempt. Source inputs are fixed by the maps; run the maintained driver on admitted ARM64 Linux with explicit `--source --inputs --target --out`. `--labels` receipts prove only selected gates.

Linux afs-build:4CPU/8GiB, Rust/cargo1.95, ext4 reused release cache, no concurrent Cargo at admission; source and outputs frozen under `/var/tmp/afs-native-workspace-20261007-r1`, shared target `/var/tmp/afs-e2e-release-20261006-r1/target`. Offline dependencies, compiler identity, profile overrides, `/dev/fuse`, sudo admission and per-gate capacity were checked and recorded. Minimum observed free space was~4.22GB, reserve1.5GiB; no capacity stop or environment repair.

[Runtime admission observations](runtime-admission-observations.json) retain the actual missing-runtime observations. They are not a complete runtime preflight PASS. [Independent final review](final-review.json) approves default-OFF **partial** publication only. Unknown failed-create state and unverified final identity refuse success; production READY, revocation/drain ACK, restart reconciliation, append/lock/mixed-mmap/watch gates and actual managed lifecycle remain open. See [slice contract](../../native-workspace-slice.md).

Previous standard/cross-node/recovery results keep their original versions. This new ELF does not inherit Owner/DFS pjdfstest, full POSIX, recovery matrix, performance or package qualification. Ordinary benchmark raw data and FAIL remain unchanged, with focused tuning deferred.
