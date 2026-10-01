# Native bind development evidence: mount transactions, references and journal maintenance

Issue: https://github.com/lelezi257/dms/issues/42. Plan: `native-bind-plan.md`.

This is an intermediate slice, not feature completion, product acceptance or performance qualification. It does not enable native exports in Node. Default configuration does not enable native admission. The optional hint hook remains inert without a sink; ordinary directory identity refresh now reconciles cached descendants after an external move. Node/FUSE/P2P integration, cross-path locks/cache/mmap, full Node/Agent lifecycle recovery and ext4 performance comparison remain outstanding.

## Tested candidate

- Current base: `78245771167643d5883491052e7cebcaba8c3be2` (original base `6bcabe8f30040bc6cc3b518bd271e7e2461e1e1d`); branch `feat/ownerfs-native-bind`. Feature worktree is isolated from the canonical checkout and the other machine's main branch.
- Rust1.95 x86_64 Linux. WSL6.6 builds/tests the controller and journal; actual mount backend is run in Linux6.8 VM A on `/dev/sdb1`, ext4 UUID `6fa5e173-b766-4c27-872b-8f40e91bed27`.
- Current VM test binary SHA256 `f8689bbb02bccbc9736a7c570d8836a2d2907cc5888bfebe56a73edc3464bc15` (directory-refresh candidate; full14-case suite is RED, exact source inputs preserved externally).
- Private mount namespaces; test data confined to uniquely created directories. Earlier successful probes checked parent namespace mountinfo byte-for-byte. The ordinary RED driver exits before its cleanup markers; the paired portable RED replay independently verifies parent mounts and disposable-data cleanup. Most covered test directories are disposable ext4 directories. One test now uses the real OwnerFs FUSE adapter and RootManager with an in-process Meta fixture; it does not run production Node/P2P or enable native policy.

## Outcomes

| Check | Result | Meaning |
| --- | --- | --- |
| Workspace event channel | 5 PASS | Bounded capacity, raw-byte names, full-queue rescan/rearm, disconnect and one-time sink installation |
| Controller regressions | 16 PASS | Idempotent export, stale identities, busy retry, foreign ownership, recycled-ID protection, bounded admission |
| Journal regressions | 10 PASS | Exclusive writer, strict decode, symlink/hardlink protection, immutable old epoch, atomic replace, uncertain directory-sync failure |
| Journal/controller transactions | 12 PASS | Pre-attach exclusive clone intent, durable ACK, unmount intent, restart without stacking, same-epoch retired-session fencing, bounded orphan cleanup, uncertain unlink/fsync and duplicate-ACK health |
| Current VM Linux backend/FUSE/old-directory semantics | RED: 13 PASS, 1 FAIL | Original cases pass; retained old FUSE dirfd/cwd do not automatically track a native directory move |
| Current library regressions | 353 PASS, 2 ignored | Main-aligned OwnerFs/DFS regression plus external-directory refresh; ignored tests remain outside this claim |
| fmt / strict all-targets Clippy | PASS | Candidate formatting and diagnostics |
| Portable VM probe | Current14-case RED in both cache controls; prior13-case PASS | Both current replays execute13 PASS/1 FAIL and separately verify unchanged parent mounts and empty disposable data; earlier13-case PASS does not qualify full semantics |
| OwnerFs FUSE/native/P2P integration and performance | NOT_RUN | Mount primitive proof is insufficient |

WSL cannot supply STATX_MNT_ID_UNIQUE on kernel6.6; the backend returns ENOTSUP. All14 current real-backend/FUSE tests are explicitly ignored for the ordinary build-host run, then explicitly executed (none ignored) by the VM probe. Kernel unique IDs are required; recyclable mountinfo IDs never substitute for ownership.

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

## Main alignment and directory semantics checkpoint

Only the isolated feature branch was rebased onto main7824577. The old event
tip b38b0d8 survives in `backup/ownerfs-native-bind-pre-main-20261001`; the
rebased event tip is6ad14a9. Canonical checkout/main were not changed. On that
rebased candidate the library352/2-ignored, native43/13-ignored, strict Clippy,
fmt and none/ownerfs/dfs checks passed. VM `mount-20261001T011309-fc242a47`
and portable `portable-20261001T011648-ec2b2fe0` each passed13/13 with binary
SHA256 `f06e6cfda95fe106a621dbd7e345ba84dde01d2a5a0f1927258b95eb77eeed20`.
Those results precede the additional full-semantics case below.

The new real-FUSE case retains a pre-bind directory fd and cwd, then moves
that directory through the same-path native export. Initial old-FUSE child
lookup returns ENOENT and `..` still resolves LEFT; the native retained fd
reads the same directory data and current RIGHT parent. A diagnostic lookup
of the new path through a retained covered-FUSE root moves the kernel alias
to RIGHT, but originally child lookup still failed. This separates kernel
alias relocation from stale backend canonical/descendant paths. Neither an
extra user lookup nor a periodic scan is an accepted correctness requirement.

A failing OwnerFs library regression reproduced the latter defect. Fresh
identity-matched directory lookup now reconciles the cached subtree through
the existing rename-path operation. Regular-file hardlink canonical selection
remains unchanged. This is explicit-lookup repair, not automatic old-reference
coherence, inode-reuse protection, remote-object lifetime or full native admission.

Current library353/2-ignored and native43/14-ignored pass; fmt, strict Clippy and
none/ownerfs/dfs checks pass. VM `mount-20261001T015705-82191594` executes14 cases:13 pass and1
fails. Forced-lookup controls now explicitly pass data, dirfd-parent and cwd-parent
assertions, then the original first-observation requirement still fails. The
actor is reaped and both owned exports are normally unmounted before assertions;
the failing suite driver exits before its separate cleanup markers. Failed
runs012136/013406 and the current partial-fix result are retained externally.

Automatic source-object/alias consistency, cache/mmap/locks, authority/Agent
drainage, Node/P2P integration and measured performance remain outstanding.
Close durability and proposed separate6.9+ feature-platform decisions remain
unanswered; no production native activation or kernel upgrade was performed.

## Directory cache and callback-sequence checkpoint

The old-reference case now has an optional, test-only request trace using the
existing process logger. It reads the old parent before trying child data,
then repeats after the normal private TTL expires, and finally runs the
explicit fresh-lookup diagnostic. All original first-observation assertions
remain, with explicit assertions for the first parent-only result before the
combined-observation assertions. `AFS_NATIVE_TRACE_DIRECTORY=1` enables JSON request output;
`AFS_NATIVE_DIRECTORY_SHARED_CONTROL=1` first uses the authenticated fixture
peer executor and its existing cache-invalidation barrier. This is not a
native admission API or deployed remote/P2P qualification. Default runtime
code and cache policy were not changed in this checkpoint.

Paired full-suite replay `directory-20261001T021638-580ff24d` uses one binary SHA256 `f8689bbb02bccbc9736a7c570d8836a2d2907cc5888bfebe56a73edc3464bc15`,
the unchanged tracked portable probe SHA256
`a497ae739fff6824d6651654da4200fddb4369dfc00f44434a517b4884d380ad`, serial tests, the same VM/kernel/ext4
and separate fresh private namespaces/data directories. Both modes execute
14 cases:13 PASS/1 FAIL. Independent post-run checks confirm unchanged parent
mountinfo and empty disposable data for both failing suites. Driver exit0
means evidence collection/cleanup succeeded; each test/probe exit101 remains
RED. Ordinary untraced run `mount-20261001T021657-f9c9a06a` independently remains13 PASS/1 FAIL.

| Diagnostic | Ordinary private cache | Existing peer shared-cache barrier |
| --- | --- | --- |
| Moving-directory GETATTR during first parent-only reads | 0 | 2 |
| Old dirfd/cwd parent observation | LEFT (stale) | ENOENT (stale backend path) |
| Native retained-dirfd parent control | RIGHT | RIGHT |
| Moving-directory GETATTR after1.2s expiry control | 3 | 3 |
| Old data/parent/cwd after expiry | ENOENT | ENOENT |
| Forced new-path lookup control | Same data, RIGHT parent/cwd | Same data, RIGHT parent/cwd |

This changes the next action: native exposure needs an explicit cache barrier
before admission, object identity/lifetime rather than stale canonical paths,
and automatic kernel alias reconciliation. Zero TTL alone is insufficient.
It is an opportunity to reconcile before an old directory traversal, not
proof that reconciliation is complete. Source inspection also shows current
nofh GETATTR runs on the FUSE receive thread; blocking recursive lookup through
that FUSE mount must not be inserted there. Worker dispatch, reference bounds,
authority validation, deleted/moved parents and concurrent native mutation
need their own regression/VM proof before integration.

Kernel source independently explains the observations: `handle_dots`/
`follow_dotdot` select the in-kernel dentry parent; default-permissions FUSE
refreshes expired attributes. `fuse_time_to_jiffies(0,0)` returns0, so zero TTL
does not create a same-jiffy grace period. An initially added20ms diagnostic
delay based on that assumption was removed before this final paired run.
Sources: [Linux6.8 namei.c](https://github.com/torvalds/linux/blob/v6.8/fs/namei.c),
[Linux6.8 FUSE dir.c](https://github.com/torvalds/linux/blob/v6.8/fs/fuse/dir.c).

Replay on an independent root-capable Linux VM with the matching binary and
tracked `acceptance/probes/ownerfs_native_mount.sh`, fresh ext4 evidence paths:

```bash
sudo env RUST_TEST_THREADS=1 AFS_NATIVE_TRACE_DIRECTORY=1 AFS_NATIVE_DIRECTORY_SHARED_CONTROL=0 bash PROBE BINARY FRESH_PRIVATE_EVIDENCE SHA256
sudo env RUST_TEST_THREADS=1 AFS_NATIVE_TRACE_DIRECTORY=1 AFS_NATIVE_DIRECTORY_SHARED_CONTROL=1 bash PROBE BINARY FRESH_SHARED_EVIDENCE SHA256
```

Expected current classification is a retained full-semantics counterexample,
not a green gate. Exact logs, source hashes and per-phase counts are external
under `directory-cache-analysis.json` and `directory-cache-inputs.json`.
fmt and strict all-targets/all-features Clippy pass. Previous353 library/43
native unit results belong to the prior directory-refresh candidate; no new
production Rust code was changed here. Full feature and performance remain
incomplete, and pending durability/platform decisions still gate activation.
