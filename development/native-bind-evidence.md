# Native bind development evidence: mount transactions, FUSE targets and retained references

Issue: https://github.com/lelezi257/dms/issues/42. Plan: `native-bind-plan.md`.

This is an intermediate slice, not feature completion, product acceptance or performance qualification. It does not enable native exports in Node. Existing OwnerFs behavior is unchanged beyond an unused module declaration. Node/FUSE/P2P integration, cross-path locks/cache/mmap, full Node/Agent lifecycle recovery and ext4 performance comparison remain outstanding.

## Tested candidate

- Base: `6bcabe8f30040bc6cc3b518bd271e7e2461e1e1d`; branch `feat/ownerfs-native-bind`. Feature worktree is isolated from the canonical checkout and the other machine's main branch.
- Rust1.95 x86_64 Linux. WSL6.6 builds/tests the controller and journal; actual mount backend is run in Linux6.8 VM A on `/dev/sdb1`, ext4 UUID `6fa5e173-b766-4c27-872b-8f40e91bed27`.
- VM test binary SHA256 `6161a0584db544cc422afb01a65cd6ccf97c7a3efe044a3d51371469960845e1` (transaction candidate before checkpoint commit; exact build inputs preserved in local manifest).
- Private mount namespaces; test data confined to uniquely created directories. Parent namespace mountinfo is byte-for-byte unchanged afterward. Most covered test directories are disposable ext4 directories. One test now uses the real OwnerFs FUSE adapter and RootManager with an in-process Meta fixture; it does not run production Node/P2P or enable native policy.

## Outcomes

| Check | Result | Meaning |
| --- | --- | --- |
| Controller regressions | 16 PASS | Idempotent export, stale identities, busy retry, foreign ownership, recycled-ID protection, bounded admission |
| Journal regressions | 10 PASS | Exclusive writer, strict decode, symlink/hardlink protection, immutable old epoch, atomic replace, uncertain directory-sync failure |
| Journal/controller transactions | 6 PASS | Pre-attach exclusive clone intent, durable ACK, unmount intent, restart without stacking, same-epoch retired-session fencing |
| VM Linux backend | 11 PASS | The prior8 plus preparation release/epoch fencing, real OwnerFs FUSE target recovery/fallback, native dirfd/cwd/shared-VMA/private-VMA busy matrix |
| Existing library regressions | 258 PASS, 2 ignored | Existing OwnerFs/DFS tests, serial run; ignored tests remain outside this claim |
| fmt / strict all-targets Clippy | PASS | Candidate formatting and diagnostics |
| Portable VM probe | PASS | `acceptance/probes/ownerfs_native_mount.sh` reproduced the same current11-test candidate, with a bounded180s child process group and identified Python3.12.3 |
| OwnerFs FUSE/native/P2P integration and performance | NOT_RUN | Mount primitive proof is insufficient |

WSL cannot supply STATX_MNT_ID_UNIQUE on kernel6.6; the backend returns ENOTSUP. All11 real-backend tests are explicitly ignored for the ordinary build-host run, then explicitly executed (none ignored) by the VM probe. Kernel unique IDs are required; recyclable mountinfo IDs never substitute for ownership.

## Failures retained

- RED missing-module/import failures preceded controller/journal/backend implementation. One Rust1.95 unresolved-import diagnostic caused compiler ICE; short diagnostics reproduced the intended missing import.
- Matching foreign mount after bind failure was incorrectly adopted by the first controller implementation. A failing regression preceded the attach-claim fix.
- Directory fsync failure after rename initially served a stale in-memory journal. A failing regression preceded the uncertain-state/reopen fix.
- First VM driver passed its mount test but failed an obsolete assertion requiring the previous environment's Node/FUSE lane to still be running. Inspection found no old lane running. No stop action was issued. Corrected driver measures actual before/after parent mounts.
- Strict Clippy found a derivable Default implementation; corrected and rerun.

## Evidence and pending contract

Raw logs/exit codes/commands and source-input hashes are local under `/home/lzc/workspace/dms/evidence/ownerfs-native-bind/20261001`. VM copies also reside under the Windows `local/native-bind-vm` run directories. Raw failed attempts are retained, not relabeled as passes. Private keys and GitHub credentials are not repository artifacts.

Durability decision remains pending: current acceptance requires successful OwnerFs close to synchronize, while the older RFC uses native ext4 plain-close semantics. No native Node activation is permitted until the selected contract is explicit. The journal now brackets attach/unmount transactions and helper restart reconciliation. A persisted claim still requires independently supplied current Root authority and fresh physical kernel reobservation. Native admission also rechecks effective mount policy. These tests preserve the same live mount namespace and parent; Node/FUSE daemon death, namespace/boot loss, Agent drainage and backing reclamation remain unproved.

## Transaction checkpoint

New controller interfaces are `with_journal`, `reconcile`, and backend `bind_journaled`, `adopt_verified_claim`, `verify_policy`; Linux `prepare_recovery` reconstructs the hidden covered directory through a nonrecursive detached parent clone. `Unmounting` records durable normal-unmount intent before the syscall. A helper that exits before attach returns to FUSE_READY; exit after attach adopts only the exact unique clone claim without stacking; exit after normal unmount recovers DETACHED export state without recreating it. DETACHED alone never proves authority fencing, managed Agent drainage or permission to reclaim backing data.

Journal format is version2, preserving retired Home/session identities across restart. Version1 prototype files are rejected and preserved rather than silently discarding fencing history. Native exports have not been activated in Node in either prototype. Prepared descriptor release is now explicit and verified; Node must invoke it only at its resolved lifecycle boundary. Orphan temp maintenance still requires lifecycle work.

Two actual VM RED cases preceded fixes: a foreign self-bind preserved directory dev/ino and was wrongly accepted as a covered target; comparison with the parent unique mount ID now rejects it. Recovery under a newly requested readonly/noexec policy initially admitted an old RW export; effective policy verification now rejects admission while preserving verified cleanup ownership. Failed and passing run directories are retained. A Windows PowerShell stderr handling failure initially interrupted host capture of a deliberate crash test; the guest driver now captures its own combined log and reports the actual SSH test exit separately.

Current final VM run: `mount-20260930T191822-9d890555`; all8 executed, none ignored; test data empty and parent mountinfo unchanged. WSL final transaction run contains32 passed (16 controller,10 journal,6 transaction) plus8 explicitly ignored VM-only tests. Strict all-targets/all-features Clippy passed. Existing library regression258 passed/2 ignored remains the transaction implementation run, not a claim about pending Node integration.

## FUSE-target and retained-reference checkpoint

`LinuxMountBackend::release_prepared` never unmounts or deletes data. It refuses an attached claim, an observed foreign mount, a changed target or a different current epoch/spec; normal unmount permits pin release and same-name re-preparation. Repeated release with no preparation is harmless. A new epoch's preparation survives a delayed old release. The API has not yet been connected to Node lifecycle.

The real-FUSE test creates `/ownerfs/agent1` through the production OwnerFs FUSE callback and RootManager, then uses the returned trusted grant/data directory to prepare the mount. Covered FUSE dev/ino and native ext4 dev/ino are measured separately. An existing transition file remains visible after native attachment; native-created data stays in the same source and is read through FUSE after normal unmount. A fresh backend re-prepares the covered FUSE inode through the nonrecursive parent clone while the daemon is alive. This is a mechanism combination test with an in-process Meta fixture, not executable Node deployment, P2P, legacy cached-fd/mmap coherence or performance qualification.

Four separately tracked child processes hold native dirfd, cwd, shared VMA and private VMA respectively. Each normal unmount returns EBUSY while its actor lives and succeeds after reaping that exact actor. Mapping actors close the opened data fd and check that no duplicate data fd remains, proving VMA-only busy retention. These are ext4 mapping/lifecycle controls, not cross-path FUSE/native mmap coherence tests.

Current exact VM candidate: `mount-20260930T194257-8875763d`, binary SHA256 `6161a0584db544cc422afb01a65cd6ccf97c7a3efe044a3d51371469960845e1`; portable replay `portable-20260930T194519-c215014f`. Both execute11/11 (none ignored), leave temporary test data empty and preserve parent mountinfo. WSL regressions32 passed/11 VM-only ignored; fmt and strict all-targets/all-features Clippy passed. One prior host UNC binary availability check failed before starting any VM test; its failed staging directory is retained, and no PASS is attributed to that attempt.

Pending platform decision: the accepted native feature must resolve transition cache/mapping semantics. Ordinary direct I/O avoids FUSE data pages but shared mmap is disabled by default. The vendor contains FUSE passthrough behind ABI7-40; Linux6.9 introduced this interface. A separate feature environment and capability-gated implementation are proposed, not yet approved, built or verified. Current6.8 and formal environment baselines remain unchanged. Sources: <https://docs.kernel.org/filesystems/fuse/fuse-io.html>, <https://docs.kernel.org/filesystems/fuse/fuse-passthrough.html>, <https://raw.githubusercontent.com/torvalds/linux/v6.9/include/uapi/linux/fuse.h>. Passthrough existence alone does not establish coherence, permission/lock lifetime or Node recovery.
