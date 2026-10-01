# Native bind development evidence: mount transactions, references and journal maintenance

Issue: https://github.com/lelezi257/dms/issues/42. Plan: `native-bind-plan.md`.

This is an intermediate slice, not feature completion, product acceptance or performance qualification. It does not enable native exports in Node. Default OwnerFs behavior is unchanged; the optional post-reply hint hook does not enable native admission. Node/FUSE/P2P integration, cross-path locks/cache/mmap, full Node/Agent lifecycle recovery and ext4 performance comparison remain outstanding.

## Tested candidate

- Base: `6bcabe8f30040bc6cc3b518bd271e7e2461e1e1d`; branch `feat/ownerfs-native-bind`. Feature worktree is isolated from the canonical checkout and the other machine's main branch.
- Rust1.95 x86_64 Linux. WSL6.6 builds/tests the controller and journal; actual mount backend is run in Linux6.8 VM A on `/dev/sdb1`, ext4 UUID `6fa5e173-b766-4c27-872b-8f40e91bed27`.
- VM test binary SHA256 `6ed5e1e1dab827c686aa3430159cd2d82bd4c04d3aec132612e592a6cb63b06e` (post-reply event candidate before checkpoint commit; exact build inputs preserved in local manifest).
- Private mount namespaces; test data confined to uniquely created directories. Parent namespace mountinfo is byte-for-byte unchanged afterward. Most covered test directories are disposable ext4 directories. One test now uses the real OwnerFs FUSE adapter and RootManager with an in-process Meta fixture; it does not run production Node/P2P or enable native policy.

## Outcomes

| Check | Result | Meaning |
| --- | --- | --- |
| Workspace event channel | 5 PASS | Bounded capacity, raw-byte names, full-queue rescan/rearm, disconnect and one-time sink installation |
| Controller regressions | 16 PASS | Idempotent export, stale identities, busy retry, foreign ownership, recycled-ID protection, bounded admission |
| Journal regressions | 10 PASS | Exclusive writer, strict decode, symlink/hardlink protection, immutable old epoch, atomic replace, uncertain directory-sync failure |
| Journal/controller transactions | 12 PASS | Pre-attach exclusive clone intent, durable ACK, unmount intent, restart without stacking, same-epoch retired-session fencing, bounded orphan cleanup, uncertain unlink/fsync and duplicate-ACK health |
| VM Linux backend/FUSE hooks | 13 PASS | The prior12 plus real post-reply root hints, nested-directory exclusion and full-queue progress |
| Existing library regressions | 258 PASS, 2 ignored | Existing OwnerFs/DFS tests, serial run; ignored tests remain outside this claim |
| fmt / strict all-targets Clippy | PASS | Candidate formatting and diagnostics |
| Portable VM probe | PASS | `acceptance/probes/ownerfs_native_mount.sh` reproduced the same current13-test candidate, with a bounded180s child process group and identified Python3.12.3 |
| OwnerFs FUSE/native/P2P integration and performance | NOT_RUN | Mount primitive proof is insufficient |

WSL cannot supply STATX_MNT_ID_UNIQUE on kernel6.6; the backend returns ENOTSUP. All13 real-backend/FUSE tests are explicitly ignored for the ordinary build-host run, then explicitly executed (none ignored) by the VM probe. Kernel unique IDs are required; recyclable mountinfo IDs never substitute for ownership.

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

Journal format is version2, preserving retired Home/session identities across restart. Version1 prototype files are rejected and preserved rather than silently discarding fencing history. Native exports have not been activated in Node in either prototype. Prepared descriptor release is now explicit and verified; Node must invoke it only at its resolved lifecycle boundary. Explicit bounded orphan maintenance is implemented below; Node must connect it at a verified maintenance boundary.

Two actual VM RED cases preceded fixes: a foreign self-bind preserved directory dev/ino and was wrongly accepted as a covered target; comparison with the parent unique mount ID now rejects it. Recovery under a newly requested readonly/noexec policy initially admitted an old RW export; effective policy verification now rejects admission while preserving verified cleanup ownership. Failed and passing run directories are retained. A Windows PowerShell stderr handling failure initially interrupted host capture of a deliberate crash test; the guest driver now captures its own combined log and reports the actual SSH test exit separately.

Current final VM run: `mount-20260930T191822-9d890555`; all8 executed, none ignored; test data empty and parent mountinfo unchanged. WSL final transaction run contains32 passed (16 controller,10 journal,6 transaction) plus8 explicitly ignored VM-only tests. Strict all-targets/all-features Clippy passed. Existing library regression258 passed/2 ignored remains the transaction implementation run, not a claim about pending Node integration.

## FUSE-target and retained-reference checkpoint

`LinuxMountBackend::release_prepared` never unmounts or deletes data. It refuses an attached claim, an observed foreign mount, a changed target or a different current epoch/spec; normal unmount permits pin release and same-name re-preparation. Repeated release with no preparation is harmless. A new epoch's preparation survives a delayed old release. The API has not yet been connected to Node lifecycle.

The real-FUSE test creates `/ownerfs/agent1` through the production OwnerFs FUSE callback and RootManager, then uses the returned trusted grant/data directory to prepare the mount. Covered FUSE dev/ino and native ext4 dev/ino are measured separately. An existing transition file remains visible after native attachment; native-created data stays in the same source and is read through FUSE after normal unmount. A fresh backend re-prepares the covered FUSE inode through the nonrecursive parent clone while the daemon is alive. This is a mechanism combination test with an in-process Meta fixture, not executable Node deployment, P2P, legacy cached-fd/mmap coherence or performance qualification.

Four separately tracked child processes hold native dirfd, cwd, shared VMA and private VMA respectively. Each normal unmount returns EBUSY while its actor lives and succeeds after reaping that exact actor. Mapping actors close the opened data fd and check that no duplicate data fd remains, proving VMA-only busy retention. These are ext4 mapping/lifecycle controls, not cross-path FUSE/native mmap coherence tests.

Current exact VM candidate: `mount-20260930T194257-8875763d`, binary SHA256 `6161a0584db544cc422afb01a65cd6ccf97c7a3efe044a3d51371469960845e1`; portable replay `portable-20260930T194519-c215014f`. Both execute11/11 (none ignored), leave temporary test data empty and preserve parent mountinfo. WSL regressions32 passed/11 VM-only ignored; fmt and strict all-targets/all-features Clippy passed. One prior host UNC binary availability check failed before starting any VM test; its failed staging directory is retained, and no PASS is attributed to that attempt.

Pending platform decision: the accepted native feature must resolve transition cache/mapping semantics. Ordinary direct I/O avoids FUSE data pages but shared mmap is disabled by default. The vendor contains FUSE passthrough behind ABI7-40; Linux6.9 introduced this interface. A separate feature environment and capability-gated implementation are proposed, not yet approved, built or verified. Current6.8 and formal environment baselines remain unchanged. Sources: <https://docs.kernel.org/filesystems/fuse/fuse-io.html>, <https://docs.kernel.org/filesystems/fuse/fuse-passthrough.html>, <https://raw.githubusercontent.com/torvalds/linux/v6.9/include/uapi/linux/fuse.h>. Passthrough existence alone does not establish coherence, permission/lock lifetime or Node recovery.

## Journal maintenance checkpoint

`NativeMountManager::cleanup_journal_orphans` freezes the registry and root transactions, rejects unreconciled or changed physical exports, then scans at most256 private journal entries before any removal. Only strict ASCII temp names containing a validated same-boot/namespace v2 snapshot with known current specs, no future operation sequence and no missing retired-session history are candidates. Unknown, malformed, future, foreign, symlink and multiply linked files are retained; no temp snapshot is promoted. Each removal rechecks the pinned file identity and synchronizes the journal directory. This relies on the same cooperative private-control-directory writer boundary as journal store, not on Agent-owned input paths.

Actual O_PATH directory testing proves unlink can succeed while directory fsync returns EBADF: subsequent maintenance and registration reject EIO until reopen. A new RED regression also reproduced a duplicate registration returning old FUSE_READY after another root's journal rename failure; duplicate ACK now checks global journal health. These tests do not authorize backing deletion or resolve old-boot/namespace recovery.

Current WSL controller/journal/transaction regressions:38 passed (16+10+12), with12 explicitly ignored VM-only cases. Fresh VM `mount-20260930T211840-851f3475` and portable replay `portable-20260930T211930-d6739656` each execute12/12, none ignored, preserve parent mounts and leave disposable test data empty. Exact binary SHA256 `f1152c87afdd64d91c23975a3a3b22dda176dd74dc0f0e36f46add5e1acceec4`. fmt and strict all-targets/all-features Clippy passed. Earlier32/11 and37/12 evidence remains historical; unchanged library258/2-ignored evidence remains its earlier scoped run.

## Lock-owner design counterexample

Tracked reproduction: `acceptance/probes/ownerfs_native_lock_owner.py`, invoked with a fresh absolute ext4 evidence directory. VM run `owner-lock-20260930T203148-aeeed453` used Linux6.8.0-142. The native process can downgrade its own POSIX write lock through its second fd. A child proxy inheriting the same file description cannot do that: POSIX SETLK with the parent's supplied l_pid returns EAGAIN, as does an OFD read-lock proxy. This is `observed_design_rejection`, not an OwnerFs locking PASS and not proof that every design is impossible. The probe retains its data and exact process/kernel/result identities.

Consequently a Home helper or OFD proxy alone cannot preserve same-process lock conversion/unlock across native and an old local FUSE path. The final matrix must include this case. Linux6.9 passthrough data-I/O/mmap support by itself is not a lock solution; its FUSE lock handlers still use FUSE locking paths (source: <https://github.com/torvalds/linux/blob/v6.9/fs/fuse/file.c>). No production lock mechanism, platform change, Node activation or full-feature/performance qualification is claimed here.

## Post-reply workspace event checkpoint

OwnerFs can now install a `WorkspaceEventSender` once. Default constructors have
no sink; installing this metadata hint sink does not change cache/permission
policy or native eligibility. The FUSE mkdir adapter publishes only after its
successful reply and only for a direct OwnerFs root child. It preserves the raw
component and opaque backend inode; the hint has no grant, epoch, physical path
or mount claim. The forthcoming worker must independently resolve current
authority and prepared directory identities. No callback executes a mount or
waits for a worker response.

Capacity is limited to4096 hints. Overflow retains the queued hint and marks a
required inventory rescan; `begin_rescan` clears the flag before an authoritative
scan so concurrent overflow re-arms it. A failed scan must remain pending in the
worker. Disconnect is observable through the sender's error/rescan state. The
channel is not itself a durable event journal or a recovery/admission proof.

Missing APIs first failed compilation. The real VM test then failed with a
missing post-reply hint (12 pass/1 fail) before the FUSE hook was implemented.
The final kernel test leaves the consumer idle through two root mkdirs, nested
mkdir and file writes; both root requests complete, only root hints are emitted,
and overflow requests rescan. A subsequent FUSE create provides the callback
order barrier before inspecting the post-reply flag. A first source-edit guard
matched another struct's similar field and stopped before writing OwnerFs;
the edit was narrowed to the actual OwnerFs declaration, preserving that failure.

Current WSL regressions43 passed (5 event+16 controller+10 journal+12 transaction),
with13 explicitly ignored VM-only tests. Existing library258 passed/2 ignored
was rerun on this candidate. fmt and strict all-targets/all-features Clippy
passed. VM `mount-20261001T010728-773145a1` and portable `portable-20261001T010750-7c26c87e` each ran13/13, none ignored;
parent mounts were unchanged and disposable test data was empty. Binary SHA256
`6ed5e1e1dab827c686aa3430159cd2d82bd4c04d3aec132612e592a6cb63b06e`. Source inputs and all RED/GREEN logs remain external.

This connects notification to the real FUSE adapter; it is not the independent
Node manager/authority worker or full native feature. Native bootstrap,
capability/cache/lock contracts, P2P integration, Agent/authority lifecycle,
daemon/boot-loss recovery and performance remain required.
