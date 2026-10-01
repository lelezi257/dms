# Native bind development evidence: mount transactions, references and journal maintenance

Issue: https://github.com/lelezi257/dms/issues/42. Plan: `native-bind-plan.md`.

This is an intermediate slice, not feature completion, product acceptance or performance qualification. It does not enable native exports in Node. Default configuration does not enable native admission. The optional hint hook remains inert without a sink; ordinary directory identity refresh now reconciles cached descendants after an external move. Node/FUSE/P2P integration, cross-path locks/cache/mmap, full Node/Agent lifecycle recovery and ext4 performance comparison remain outstanding.

## Accepted native contract (2026-10-01)

The user accepted the [native profile and concrete cases](../docs/architecture/ownerfs-native-access.md): management verifies the export in the final Agent namespace before starting the Agent; cross-path file visibility is close-to-open; retained FUSE directory references need not immediately track native rename/delete. Native plain close is a visibility endpoint, while explicit applicable sync establishes durability. Ordinary FUSE-only/DFS defaults remain unchanged. Earlier strong directory/cache diagnostics retain their actual assertions and results; they are boundary evidence, not product acceptance passes. Production admission, applicable cache/locks/lifecycle and performance work remain incomplete.

The candidate and outcomes below describe recorded historical checkpoints, not qualification against the revised profile.

## Tested candidate

- Current base: `78245771167643d5883491052e7cebcaba8c3be2` (original base `6bcabe8f30040bc6cc3b518bd271e7e2461e1e1d`); branch `feat/ownerfs-native-bind`. Feature worktree is isolated from the canonical checkout and the other machine's main branch.
- Rust1.95 x86_64 Linux. WSL6.6 builds/tests the controller and journal; actual mount backend is run in Linux6.8 VM A on `/dev/sdb1`, ext4 UUID `6fa5e173-b766-4c27-872b-8f40e91bed27`.
- Current VM test binary SHA256 `cbce155edfffe9e758004b0708a312509b1176a3441999695d32e4c5738de02c` (19 cases; read-only directory repair and deterministic concurrency counterprobes; full semantics remains RED).
- Private mount namespaces; test data confined to uniquely created directories. Earlier successful probes checked parent namespace mountinfo byte-for-byte. The ordinary RED driver exits before its cleanup markers; the paired portable RED replay independently verifies parent mounts and disposable-data cleanup. Most covered test directories are disposable ext4 directories. One test now uses the real OwnerFs FUSE adapter and RootManager with an in-process Meta fixture; it does not run production Node/P2P or enable native policy.

## Outcomes

| Check | Result | Meaning |
| --- | --- | --- |
| Workspace event channel | 5 PASS | Bounded capacity, raw-byte names, full-queue rescan/rearm, disconnect and one-time sink installation |
| Controller regressions | 16 PASS | Idempotent export, stale identities, busy retry, foreign ownership, recycled-ID protection, bounded admission |
| Journal regressions | 10 PASS | Exclusive writer, strict decode, symlink/hardlink protection, immutable old epoch, atomic replace, uncertain directory-sync failure |
| Journal/controller transactions | 12 PASS | Pre-attach exclusive clone intent, durable ACK, unmount intent, restart without stacking, same-epoch retired-session fencing, bounded orphan cleanup, uncertain unlink/fsync and duplicate-ACK health |
| Current VM Linux backend/FUSE/directory mechanism | RED: proposed unlinked repair17 PASS/2 FAIL; pinned-attribute control14 PASS/5 FAIL with repair | Serial directory probes pass, but deterministic getcwd window and production old-directory case fail |
| Current library regressions | 353 PASS, 2 ignored | Main-aligned OwnerFs/DFS regression plus external-directory refresh; ignored tests remain outside this claim |
| fmt / strict all-targets Clippy | PASS | Candidate formatting and diagnostics |
| Portable VM probe | Current19-case controls plus repeated counterexample, RED | One binary: repair-off13/6; repair-on14/5 without unlinked repair,17/2 with it; all independently verify cleanup |
| OwnerFs FUSE/native/P2P integration and performance | NOT_RUN | Mount primitive proof is insufficient |

WSL cannot supply STATX_MNT_ID_UNIQUE on kernel6.6; the backend returns ENOTSUP. All19 current kernel/backend/FUSE/prototype tests are explicitly ignored for the ordinary build-host run, then explicitly executed (none ignored) by the VM probe. Kernel unique IDs are required; recyclable mountinfo IDs never substitute for ownership.

## Failures retained

- RED missing-module/import failures preceded controller/journal/backend implementation. One Rust1.95 unresolved-import diagnostic caused compiler ICE; short diagnostics reproduced the intended missing import.
- Matching foreign mount after bind failure was incorrectly adopted by the first controller implementation. A failing regression preceded the attach-claim fix.
- Directory fsync failure after rename initially served a stale in-memory journal. A failing regression preceded the uncertain-state/reopen fix.
- First VM driver passed its mount test but failed an obsolete assertion requiring the previous environment's Node/FUSE lane to still be running. Inspection found no old lane running. No stop action was issued. Corrected driver measures actual before/after parent mounts.
- Strict Clippy found a derivable Default implementation; corrected and rerun.

## Evidence and pending contract

Raw logs/exit codes/commands and source-input hashes are local under `/home/lzc/workspace/dms/evidence/ownerfs-native-bind/20261001`. VM copies also reside under the Windows `local/native-bind-vm` run directories. Raw failed attempts are retained, not relabeled as passes. Private keys and GitHub credentials are not repository artifacts.

At this historical checkpoint the durability decision was pending. The accepted native contract above now separates native close visibility from explicit-sync durability; ordinary FUSE-only/DFS policies remain unchanged. This contract decision does not establish production native admission or recovery qualification. The journal now brackets attach/unmount transactions and helper restart reconciliation. A persisted claim still requires independently supplied current Root authority and fresh physical kernel reobservation. Native admission also rechecks effective mount policy. These tests preserve the same live mount namespace and parent; Node/FUSE daemon death, namespace/boot loss, Agent drainage and backing reclamation remain unproved.

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

## Automatic directory-reference mechanism checkpoint

Tracked test-only module `tests/ownerfs_native_linux/directory_reference_probe.rs`
uses the real OwnerFs backend and fixed in-process Meta fixture, but a separate
minimal read-only FUSE adapter. It is not production `AfsFuse`, a directory RPC
protocol, a native admission API or a full filesystem implementation. The custom
adapter replies with zero TTL and pins exactly one backing directory plus its
source root and covered FUSE workspace root before attachment.

After native rename, a worker reads the backing descriptor's current path,
checks its pinned dev/ino and root membership, then performs a new-name lookup
through the already-held covered FUSE root before replying to the caller's
GETATTR. That lookup uses the existing OwnerFs subtree-reconciliation fix and
allows the kernel to move the old FUSE alias. The receive thread remains free
to answer helper requests. Recursive helper identification uses only its live
thread ID; daemon-process-wide exemption was removed. Actual FUSE header TIDs
match the two worker TIDs in the final VM trace. Linux source uses
`task_pid(current)` for this header field:
[Linux6.8 FUSE dev.c](https://github.com/torvalds/linux/blob/v6.8/fs/fuse/dev.c).

The caller opens old FUSE dirfd/cwd before bind. Native moves the directory
left→right, reuses the old name for a new directory with different bytes,
and the caller's FIRST operation is a parent-only read. No caller new-name
lookup, sleep, scan or inotify event is needed to repair that observation.
Only after it completes does the test look up the replacement name as a
positive control. It then removes that replacement and moves the original
directory right→left. Both phases compare parent, cwd and child data with
the original native directory fd, preserving the original backing identity.

Initial missing `open_flags` adapter argument was a compile error, corrected
before behavioral claims. With automatic repair disabled, the no-replacement
case returned ENOENT; old-name reuse strengthened the control to ESTALE. With
repair enabled, FIRST old dirfd/cwd reads observe RIGHT and the original data;
the second move observes LEFT and the same original data. The replacement-name
control reads its different bytes. Single-case RED/GREEN logs remain historical
and are not substituted for the final TID-only candidate.

Final paired full replay `directory-20261001T024545-fced027f`, binary SHA256 `c8329a13482634ea1c087deef54a2a2dbadd5d36a11ca43537b0bd2980965270`, executes15
cases with the unchanged tracked portable probe. Mode0 disables the prototype
and has13 PASS/2 FAIL. Mode1 enables it and has14 PASS/1 FAIL. The remaining
failure is still the production OwnerFs/AfsFuse old-directory case. Each replay
returns test/probe101; the collection driver returns0 only after independently
checking unchanged parent mountinfo and empty disposable data. The actor is
reaped and exact worker threads are joined before owned normal unmount and
pin release. Raw logs, helper TIDs, source-input hashes and summary live under
`directory-worker-analysis.json` / `directory-worker-inputs.json` externally.

Replay modes through the existing VM probe, in serial fresh ext4 lanes:

```bash
sudo env RUST_TEST_THREADS=1 AFS_NATIVE_REFERENCE_REPAIR=0 bash PROBE BINARY FRESH_DISABLED_EVIDENCE SHA256
sudo env RUST_TEST_THREADS=1 AFS_NATIVE_REFERENCE_REPAIR=1 bash PROBE BINARY FRESH_ENABLED_EVIDENCE SHA256
```

This demonstrates a viable kernel mechanism for the tested named-directory
moves, not complete object/alias semantics. Deleted directories and parents,
source-root changes, nested moves, concurrent native mutation, credentials,
lookup/forget and open-handle pin bounds, crash/epoch/fencing, directory RPCs,
cache/mmap/locks and performance remain required. Production dispatch,
reference registry and cache/admission barriers are not wired. The adapter's
limited helper/handle logic and read-only flush are not reusable production
contracts. No Node native activation or platform/durability change occurred.
fmt and strict all-targets/all-features Clippy passed; previous production
library/unit counts remain their earlier scope.

## Deleted-directory counterexample and pinned-attribute control

The test-only adapter now includes deletion after a native move, plus an
in-place deletion control. The caller retains the old FUSE dirfd/cwd and an
ext4 directory fd before bind. Native deletes the data file and directory
before the caller's first observation. No fresh-name caller lookup occurs.
After the exact actor exits and workers join, owned mounts and pins are
released before semantic assertions. No product interface changes follow.

In the moved/deleted case, the native fd still reads RIGHT through `..`;
`fstat` succeeds with directory mode0755 and nlink0; child lookup is ENOENT.
Both ordinary path-based attributes and the earlier named-directory worker
instead return ENOENT for the old FUSE parent traversal and fstat. The
worker explicitly rejects nlink0 and cannot repair via a nonexistent name.

`AFS_NATIVE_PINNED_DIRECTORY_ATTR=1` is an explicit test-only diagnostic:
it replies from the pinned object's metadata, with the retained dev/ino
checked, for this single deleted directory. Fstat then matches ext4 and
child lookup remains ENOENT, but the old FUSE dirfd/cwd parent is LEFT,
while ext4 is RIGHT. The in-place-deletion control matches ext4 with this
attribute control: both parent reads are LEFT, fstat matches, child is
ENOENT. This isolates object survival from kernel parent relocation;
correct fd attributes alone cannot close the moved/deleted case.

Final17-case source is tested with one matching binary in four serial fresh
VM namespaces. Runs `directory-20261001T025945-dcb3a7cd` (pin attributes0)
and `directory-20261001T025902-f64d89e9` (pin attributes1) use repair0/1.
Results respectively13 PASS/4 FAIL,14 PASS/3 FAIL and14 PASS/3 FAIL,
15 PASS/2 FAIL. All execute every case, none ignored. Tests/probe exit101;
driver0 only records successful collection plus independent unchanged
parent mountinfo and empty disposable data. Production old-directory
case and moved/deleted mechanism case remain RED with both controls.
External `deleted-directory-analysis.json` / `deleted-directory-inputs.json`
record observations, source hashes, binary identity and raw evidence.
Earlier16-case runs025508/025751 are retained as intermediate reproduction,
not results for this final17-case candidate. Format, integration-test build,
strict all-targets/all-features Clippy and format check pass on final inputs.

Reproduce the pinned control by adding `AFS_NATIVE_PINNED_DIRECTORY_ATTR=1`
to each serial portable invocation above, using fresh evidence directories.
Omitting it retains the original path-based attribute behavior. This flag
exists only in the probe adapter. Root identity/authority, credentials,
concurrency, bounded object leases, production/P2P integration, locks/cache,
lifecycle and performance remain unqualified. Deleted objects are not
excluded from the goal; a production solution must preserve their current
parent relationship without resurrecting a visible pathname.

## Unlinked alias repair: serial mechanism and rejected atomicity

The separate read-only test adapter adds `AFS_NATIVE_UNLINKED_ALIAS_REPAIR=1`,
used only together with repair and pinned metadata flags. It pins the deleted
directory, opens its actual `..` through that fd, checks current parent identity
and source-root membership, then looks up the covered FUSE parent off the
receive thread. One live helper TID, exact parent inode and fixed fixture name
`moving` receive a zero-TTL entry for the pinned deleted inode. Ordinary
lookups retain the real backend result. No backing file/name is created.
After that lookup completes, synchronous entry invalidation unhashes the alias
before the original GETATTR reply. The fixture name is known; this is not a
generic parser of `/proc` deleted-name suffixes or an authorization design.

Serial named, moved/deleted and in-place/deleted cases now match their native
fd oracle: parent and child reads, nlink0 attributes, relative fd path with
` (deleted)`, unreachable getcwd, and missing visible name. These observations
are useful kernel mechanism evidence but do not qualify this two-step repair.

The coalesced-lookup counterprobe holds the helper's entry reply while a
separate thread stats the same absent name. Stationary parent attrs alone
have60s TTL in this counterprobe; the moving object/entry stays zero TTL.
Actual observer stack is captured before the reply: `d_alloc_parallel`,
`__lookup_slow` and statx. The thread coalesces on the pending helper lookup,
then returns ENOENT. The exact observer is joined before owned cleanup.
The first assertion incorrectly expected the non-inlined `d_wait_lookup`
symbol. Ubuntu's kernel reports its caller; the corrected check requires
both the observed wait and pending-path kernel stack. That earlier harness
RED is retained, not relabeled as a semantics failure. Relevant source:
[Linux6.8 dcache](https://github.com/torvalds/linux/blob/v6.8/fs/dcache.c).

**The deterministic getcwd counterprobe rejects production use.** Two exact
Python children establish FUSE cwd and physical ext4 cwd before bind. After
native move+delete, the worker pauses after alias relocation but BEFORE entry
invalidation and asks each child for getcwd. FUSE returns `right/moving`,
whereas ext4 returns ENOENT. This is a real scheduler-visible interval, widened
by an explicit test handshake, not a timeout assumption. Receive-thread
availability and zero TTL do not gate this kernel-only operation. Both child
PIDs are identified and reaped before owned cleanup; later original actor
observations match only after the interval closes. Do not remove this
assertion, exclude concurrent cwd, or claim faster invalidation guarantees
correctness. Kernel implementation:
[Linux6.8 d_path/getcwd](https://github.com/torvalds/linux/blob/v6.8/fs/d_path.c).

Python initially tried to search its FUSE cwd for imports and hit the minimal
adapter's unsupported readdir. Only the observer harness now uses Python `-I`
to remove cwd import searching. That run is a retained startup failure, not
functional evidence. The fixed harness reproduces the semantic mismatch.

Earlier19-case binary controls: `directory-20261001T031454-a4ddbc85` has the
unlinked repair flag off,13 PASS/6 FAIL with repair0 and14 PASS/5 FAIL with
repair1. Enabled runs `directory-20261001T031336-5c0703f2` and repeated
`directory-20261001T031514-d27f2099` each have13 PASS/6 FAIL and17 PASS/2 FAIL.
One identical binary is used throughout. Enabled failures are production
old-directory handling and the new getcwd window. Every case executes,
none ignored; tests/probe exit101. Collection driver0 requires independently
unchanged parent mountinfo and empty disposable test data. Inputs and raw
evidence are indexed externally by `unlinked-alias-analysis.json` and
`unlinked-alias-inputs.json`. Format/build/strict Clippy/fmt-check pass;
library/unit counts remain their earlier production-change scope.

Reproduce by adding `AFS_NATIVE_PINNED_DIRECTORY_ATTR=1` and
`AFS_NATIVE_UNLINKED_ALIAS_REPAIR=1` to the serial portable commands above.
Omitting the latter retains the pinned-attribute negative control. These
flags affect only the custom probe. No production adapter, Node admission,
RPC, kernel installation or accepted contract was changed.

The next design must make parent relocation and deleted-name visibility
atomic for relevant observers. Standard6.8 notification enumeration has
invalidation/delete but no atomic reparent notification:
[Linux6.8 FUSE UAPI](https://github.com/torvalds/linux/blob/v6.8/include/uapi/linux/fuse.h).
This source inspection plus counterprobe rejects this implementation, not
every possible native-bind architecture. Kernel/bridge alternatives require
separate feasibility evidence; none is currently approved or implemented.
Current full-goal cache, mixed lock-owner, directory leases/P2P, authority,
lifecycle, durability decisions and performance requirements remain open.

Final visible-name check selects the actually deleted name in each fixture:
`left/moving` for in-place deletion and `right/moving` after a move. This
strengthens the in-place absence control instead of checking only RIGHT.
Final matching-binary19-case runs `directory-20261001T031823-5175d363`
(unlinked repair0) and `directory-20261001T031801-623c3ec3` (repair1) reproduce
13/6,14/5 and13/6,17/2 respectively. Serial deleted-name controls and
coalesced observer pass with repair; the deterministic cwd interval remains
RED. Cleanup and final format/build/strict-Clippy/fmt-check all complete.
Earlier repeated runs remain tied to their earlier SHA, not reassigned to
this final candidate. Final raw evidence/input hashes are indexed by
`unlinked-alias-final-analysis.json` / `unlinked-alias-final-inputs.json`.

## First getcwd before any repair callback (historical diagnostic)

The 20-case binary SHA256 `017932c8613bdf3d7a6e3bee623347accd604379c57461313caa578df4dbc10f`
adds `privileged_native_move_first_getcwd_probe`. Native move and old-name
replacement occur before either prepared cwd actor first asks for its path.
The FUSE actor returns `left/moving`, the native actor `right/moving`;
moving-directory GETATTR count stays 4 before and after. Thus this operation
cannot be repaired by waiting for a FUSE callback that it does not issue.
The unchanged strong-equivalence assertion fails after owned actor cleanup.

Run `directory-20261001T032414-5552aea2` records repair-off 13 PASS/7 FAIL and
repair-on 17 PASS/3 FAIL, none ignored; test exit101 and collection exit0
are distinct. Parent mounts are unchanged and disposable data empty in both.
fmt/build/strict Clippy/fmt-check succeeded. Raw copies and input hashes are
under external `vm-first-getcwd/` and `first-getcwd-inputs.json`.
With the subsequently user-accepted native profile this first-getcwd mismatch
is boundary evidence, not a current requirement for immediate old-directory
tracking and not a product acceptance pass. No production policy was changed.
