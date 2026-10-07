# OwnerFs workspace bind mount: real FUSE core integration

2026-10-07. **PASS in the bounded component scope below.** [Plan and missing coverage](../../ownerfs-workspace-bind-core-slice.md). This supplements G2.12; G1 historical8/8, G2 task counts and default OFF remain unchanged.

## Identity and actual checks

| Identity | Value |
| --- | --- |
| Source base | 165d13f07b84092f5da5c3ad44bed1016b496378 |
| Compiler inputs | 157 files, [frozen map](compiler-inputs.json), SHA256 6dd63d4681db557af2b0105d69d381572dfc92f300790745c523e59760a91735 |
| Only compiler-input change from base | test-only src/node/vfs/ownerfs/native_home_fuse_fixture.rs, SHA256 c9b3976473e0f4361e161ba5bc2ad5555cc18c870ff8dcd29e0be150f172e7ea |
| Historical test preserved | Original 6,442-byte prefix unchanged, SHA256 2b7ad4414fc842d03cf15b80108d3abb6c9f5e1a83ca375eb01393dadda8e976 |
| Actual test ELF | SHA256 0150c33698133f1f3c41bd1c8d1d729de346a320d7efdda25de7927d96e9d0c8, 27,469,240 bytes; [Cargo-derived path and ldd](test-artifact.json) |
| Build environment | afs-build, Linux ARM64, guest ext4; [fixture preflight](fixture-preflight.json), [build preflight](preflight.json) |

[Source proof](source-proof.json): Linux fmt, affected release test compilation and strict workspace/all-target/all-feature Clippy all exit0; input hashes unchanged. The proof also lists pre-existing service binaries for identity, **not newly built/deployed services**: no release-build label was selected. Production/config/vendor inputs did not change.

[Exact runtime command](runtime-command.json) invokes one ignored test under root in an externally created private mount namespace, no runc and no shell bind operation. [Actual result](runtime-result.json), [raw stdout](runtime.stdout), [stderr](runtime.stderr): **1 passed, 0 failed, 585 filtered out**, exit0. [Postcheck](runtime-postcheck.json): all11 checks PASS; actual owned child gone, fixture removed, [full post-run mountinfo](mountinfo-after.json) matches the original26 rows; no pre-registered protected processes (empty set) or AFS/build processes are present, all157 source hashes unchanged, 40,319,123,456 bytes free. No environment install/repair/resize and no lazy/forced fixture cleanup.

## What the test proves

- Actual authorized Home backing directory `/var/tmp/.tmpOjHYwe/root-6e6174697665-e1` (ext4, device64769/inode547085) is mounted by WorkspaceBindMount onto `/var/tmp/.tmpOjHYwe/core-bind-fuse/native`, the corresponding first-level directory beneath a real OwnerFs FUSE root. It does not bind FUSE to itself. Core claimed and independently observed mount identity agree (mount131, outer FUSE125).
- Full tiny content written/synced/closed through bind is freshly opened/read through a retained pre-cover FUSE directory view; the reverse direction also matches. This checks local close-to-open between those controlled views.
- An owned ordinary child successfully holds cwd on the bind target. Normal detach returns EBUSY; claimed and actual mount identity131 remain. Child118058 is released and actually waited with exit0; normal detach restores the original FUSE target identity and outer mount125.
- Held descriptors are dropped, outer FUSE is explicitly unmounted with flags0 before session join, no fixture mount remains and the owned temporary directory is removed. Assertion failure retains data and prevents fuser's implicit lazy fallback; only the owned child handle is cleaned up.

[Independent static review](static-review.json) and [independent runtime evidence review](runtime-review.json) APPROVE the bounded result. [SHA256 manifest](SHA256SUMS) binds the portable packet. Raw Cargo output is retained; no test source snapshot, ELF or transport archive is copied into Git.

## Boundaries and next item

This is a test-only Meta fixture in a private namespace. It does not implement a standalone Node switch, ordinary-host mount visibility, multi-workspace lifecycle, general revoke/reference drain or production READY. Existing mixed-path append/classic-lock/watch failures remain unchanged. It is neither full ON/G2.12 closure, POSIX certification nor G2.13 performance acceptance. No new package/deployment or old runtime PASS is inferred.

The naming/ownership repair was already [published165d13f](https://github.com/lelezi257/dms/commit/165d13f07b84092f5da5c3ad44bed1016b496378): core is one ownerfs/bind_mount.rs, runc remains its separate adapter, legacy config names remain compatible and OFF. Next continue the existing G2.12 Node admission/lifecycle boundary before G2.13; do not widen host visibility simply by removing unshare.

The stale remote next-entry was corrected by reading existing [read](../20261007-owner-remote-small/README.md), [write](../20261007-owner-remote-write-small/README.md), [delete](../20261007-owner-remote-delete-small/README.md) and [DFS sync-read](../20261007-dfs-sync-read-small/README.md) evidence. They were **not rerun** and no extra completed-task count is added. Historical versions/failures/links are retained.

Publication-only [first link/index check failed](publication-checks.pre-manifest.json) because SHA256SUMS had not yet been created; [failure and correction](manifest-first-attempt.json) are retained. The index generation order was corrected, then only publication checks repeated; no product test/build rerun. [Final publication check](publication-checks.json).
