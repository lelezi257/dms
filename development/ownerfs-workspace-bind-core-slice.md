# OwnerFs workspace bind mount: real FUSE core integration

2026-10-07. Bounded G2.12 functional supplement after ownership repair165d13f. No production code/config/vendor change; default OFF/G1 historical8/8/G2 counts unchanged.

## Evidence-driven entry correction

Remote B-Home read/write and delete already ran, and synchronized DFS one-writer/two-readers already ran. Reuse [remote read](evidence/20261007-owner-remote-small/README.md), [remote write](evidence/20261007-owner-remote-write-small/README.md), [remote delete](evidence/20261007-owner-remote-delete-small/README.md), [DFS synchronized read](evidence/20261007-dfs-sync-read-small/README.md). The ownership repair's proposed next-entry repeated an older preflight and is superseded here; no repeated execution or new performance PASS. Remote deletion G2.16 remains completed; ratios and qualification failures retain their original identity.

## Missing coverage and plan

Existing four core physical tests use ordinary backing/target directories. The historical native_home_real_covered_root_lifecycle uses actual OwnerFs FUSE and Home authority but invokes shell mount --bind instead of the extracted core. Preserve it unchanged; add one exact ignored test in native_home_fuse_fixture.rs using the same fake-Meta fixture and existing core API, no runc.

1. Preflight once on ARM64 Linux: /dev/fuse, root/sudo, unshare, private namespace support, sh, timeout, fusermount3, exact source map/build artifact and sufficient guest ext4 capacity. Record existing processes/mounts before execution. Missing actual dependency stops this lane; do not install or repair it.
2. Create a tiny real OwnerFs FUSE workspace, acquire full local HomeExportAuthority, retain a pre-cover FUSE directory FD, use WorkspaceBindMount prepare/activate to cover the matching first-level directory. Prove physical source/target dev+ino and actual mount identity.
3. Tiny full-content write/sync/close through the bind path then fresh open/read/close through the preserved FUSE directory FD; write/sync/close through that FUSE view then independently fresh-read the bind view. No remote-RPC, full-POSIX or performance claim.
4. Ordinary short child holds cwd on the bind target and signals readiness. Normal detach must return EBUSY, retain the claim and observed mount identity. Release child, observe actual exit0, normally detach, prove original FUSE target identity/content restored, then normally end FUSE and prove no fixture mount remains. Bound wait/readiness and preserve failure evidence/data without lazy/force cleanup.
5. Linux fmt/affected test compilation/strict library-test Clippy, then exactly this new privileged test (no broad ignored run). New input map and raw commands/results; existing unaffected tests retain their original version. Independent final evidence review; update entry/index and publish Lore normally.

## Limits

A private test namespace and test-only Meta implement a controlled fixture, not a standalone Node manager. Ordinary-host visibility, general consumers/revoke/drain, shared FUSE/native append/classic locks/watch, restart reconciliation and full G2.12/13 remain pending. No attempt to widen visibility by deleting unshare; existing authority is namespace-bound and does not count as RootManager in-flight references.

## Actual bounded result

Linux frozen map6dd63d46 fmt/affected test compile/strict Clippy exit0; exact selected real-FUSE test1PASS/585 filtered. Physical ext4 Home source covers FUSE first-level native, local bidirectional fresh-open content matches; cwd EBUSY retains mount131, actual child wait0 permits normal detach and restores original FUSE mount125. Explicit normal outer umount then join, no fixture/child remaining; independent postcheck11PASS. [Raw versions, commands and limits](evidence/20261007-ownerfs-bind-core-fuse/README.md). This closes only this missing component coverage, not standalone Node lifecycle/full ON/performance. No service build/deployment, production/config/vendor change or environment repair.
