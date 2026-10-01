# Native bind development evidence: mount transactions, references and journal maintenance

Issue: https://github.com/lelezi257/dms/issues/42. Plan: `native-bind-plan.md`.

This is an intermediate slice, not feature completion, product acceptance or performance qualification. It does not enable native exports in Node. Default configuration does not enable native admission. The optional hint hook remains inert without a sink; ordinary directory identity refresh now reconciles cached descendants after an external move. Node/FUSE/P2P integration, cross-path locks/cache/mmap, full Node/Agent lifecycle recovery and ext4 performance comparison remain outstanding.

## Current checkpoint: exact command refusal and operation drainage (2026-10-01)

Source parent is `b2755eb`; the source candidate is retained at
`native-refusal-candidate-20261001T075549`. RootManager now has a separate
command-scoped refusal entry point. Before mutation it checks current Root,
epoch, Home Node/session and access generation. Closing admission and counting
admitted operations share the same root-state lock. Exact duplicate commands
resume the same refusal; changed command identity, revision or target cannot
borrow it. The token retains the complete captured RootGrant and root object.
A replacement cache object, unrelated invalidation or lost control session
cannot yield this command's current drain observation. The existing RootId-only
local invalidation API retains its uses; it is not the control-command consumer.

`native-refusal-api-red.log` retains the missing API compilation failure;
`native-refusal-behavior-red.log` retains all5 positive-path failures with the
temporary unavailable implementation. Those are missing-feature REDs, not a
claim of reproducing five previous production bugs. After implementation,
`native-refusal-green.log` passes all5 cases, including held operation guards,
eight foreign/stale/malformed targets, duplicate/payload identity, retired
cache-object rejection and control/global invalidation. The all-feature library
run `native-refusal-library.log` passes404 tests with2 existing ignored.
Strict workspace/all-target/all-feature Clippy and fmt checks pass. Compilation
with no default features, OwnerFs-only and DFS-only passes; no-feature/DFS-only
retain the two existing descriptor-helper warnings, not a per-variant lint pass.

Independent VM replay `root-refusal-20261001T075556-595f8aeb` passes the same5
cases on ext4 `/dev/sdb1` in a private mount namespace. Binary SHA256 is
`97bf876d8d742dae4ae0e12932c51d8626f9f14f268874251042eb2450d6e99c`;
raw archive SHA256 is
`5f299865ade7082865e170b3fb77cb9472a01b69ec656f1edeae649d185ca154`.
Independent archive inspection verifies all5 exact names/outcomes, input hashes,
ext4 identity, distinct namespace, unchanged parent mounts and no temporary-data
residuals. VM staging and raw evidence remain external to the source repository.

This is a process-local admission/in-flight barrier. It does not drain open
handles, dirty work, lock waiters/pins or native Agent processes, normally detach
mounts, persist pending commands/cursors or produce an ACK. The production Node
still does not call it. Those runtime integrations and full semantic/performance
gates remain mandatory; operation count zero is not a revocation completion.
`native-refusal-project.log` is overall PASS: all25 observed suite summaries
are green, including the unchanged DFS lease-expiry test that failed in prior
attempts. Its existing environmental ignores remain ignored, including the
24 privileged Linux cases not executed by the WSL project command. Earlier
failure/clock evidence is retained; this pass does not establish a WSL clock
repair. Workspace-wide higher-ABI vendored-fuser test failures remain a separate
unqualified scope; the root-project command does not execute dependency tests.

Fresh actual VM foundation replay `foundation-20261001T080203-23704e40` passes
all17 selected cases. Binary SHA256 is
`047ff16a1643a51b1cb9d8977bd75672491f0d99834b8050f679d0f2fef763e2`;
raw archive SHA256 is
`192e10148fcf8fe0b2aab115d1c153a80c01041f9a246d14e88920a1c8146775`.
Independent inspection verifies17 unchanged parent-mount pairs and17 empty
temporary directories. The7 historical stronger old-directory diagnostics are
explicitly NOT_RUN in this profile, not silently counted as passing. Foundation
scope remains mount/cache/Home teardown/local kernel-flock, not production Node,
network P2P, native POSIX locks/mmap, Agent fencing or performance qualification.

## Previous checkpoint: command feed and exact revocation ACKs (2026-10-01)

Source parent is `a463762`. Exact final source is retained in external snapshot
`native-control-timeout-candidate-20261001T073450`; prior intermediate snapshots
and all failed attempts remain preserved. This adds control prerequisites, not
a running native Node worker, managed Agent fencing or native readiness.

`PollRootCommands` is an additive OwnerRoots RPC using a pinned read view for
the current live Node session, global authority watermark and complete-revision
event prefix. Up to1024 commands are returned; oversized atomic command batches
fail instead of being split or dropped. Store event limits remain soft at a
transaction boundary. The new pinned-prefix reader avoids cloning all remaining
events before slicing, but still scans retained history and has no strict
execution-time claim. Unrelated events and receipt-only revisions advance the
processed cursor. Entity mod revisions remain unchanged for conditional updates;
the global read-view revision is distinct. Legacy watch semantics are unchanged.
The Node caller rejects wrong sessions, invalid prefix bounds, stale/unknown/
duplicate commands, invalid target facts and overflow. Same-revision commands
remain valid. Command target facts are not converted into a RootGrant. A page is
one authority observation, not an ongoing lease; applying/persisting pending
commands before cursor advance and actual process fencing remain integration work.
Compaction is a fail-closed interface boundary; no real compacted-backend or
production reconnect/reconciliation run has been qualified by this slice.

ACK admission now reads the exact command and conditionally compares that
durable entity in the ACK transaction. Root/epoch/Home session/generation must
match. `RootCommandAcked` also compares the successful ACK against its command,
so a wrong-root ACK cannot approve a commit. Exact request payload replay reads
the stored receipt even after command retirement; changed payloads or new ACKs
for a retired command fail. The legacy response acceptance time remains response
time, not a new durable timestamp. This guards ACK identity, not honesty of
drainage: a native coordinator must still derive real process/ref/normal-unmount
proof before issuing success.

Actual behavior REDs in `native-control-ack-red.log` show all3 original ACK/
barrier/replay errors. `native-control-behavior-red.log` separately proves exact
ACK replay failed after command retirement; its unsupported poll failure and
negative-only checks are not accepted poll coverage. API-missing compiler REDs
are kept separately. `native-control-green.log` records the erroneous use of an
entity mod revision as the global prefix bound (7 PASS/3 FAIL).
`native-control-empty-revision-red.log` proves receipt-only revisions stalled
the cursor (resume2 versus authority3). Final positive controls and failures
before fixes are retained rather than rewritten as passing runs.

The final `native-control-timeout-green.log` records14 source cases PASS.
`native-control-library-final.log` records399 all-feature library PASS/2 existing
ignored; strict workspace/all-target/all-feature Clippy, fmt and none/OwnerFs-only/
DFS-only compilation PASS. No-feature/DFS-only compilation retains the two
descriptor-helper warnings; strict per-variant lint has not been claimed.
`native-control-contracts.log` separately records45 native controller/event/
journal/transaction PASS.

`native-control-project.log` is overall FAIL at unchanged
`dfs_renewal_cannot_resurrect_expired_or_reassigned_authority`, with38 other Meta
cases passing. Its unexecuted remainder and doc tests are not qualified by the
preceding successes. The earlier realtime/monotonic observations remain a
plausible explanation, with no established clock fix or altered DFS assertion.
The separate failing vendored-fuser workspace/all-ABI run remains unqualified.

Initial VM control run `control-interface-20261001T072132-26cbea57` records13 PASS/
1 FAIL: the new timeout test wrongly assumed the outer timer always wins over
the equal endpoint timer. Existing main semantics preserve tonic's untyped
`CLIENT_REMOTE_STATUS` / Cancelled / `Timeout expired` as an unknown outcome.
The corrected test accepts only that exact tuple or the outer deadline code,
keeps the original2-second bound, and does not normalize runtime errors. The
VM raw log confirms the complete tuple. The harness initially failed to archive
after early exit because later logs were missing; exact guest evidence was
recollected, and the runner now archives existing logs even after early failure.
All4 later lock cases are NOT_RUN in the original failing run. Its archive
SHA256 is `7b6847046adb6a6d6af15d16658888350fdd69dd9422bd275619723585a52b53`.

Final VM `control-interface-20261001T073458-aca2f850` passes14 control cases and
all4 retained lock-authority/release cases. Library executable SHA256:
`9ade934322d37ec87f44f1561cd91291b48536f9963a5b255ea25965478c3657`.
Archive SHA256:
`57e147c1bd5cfc3acaf687599c7f32b6a02b5a3d6ae3315dd4b966457c64fbee`.
These are memory-authority/Home/kernel fixtures plus real loopback gRPC, not
production Node/network-P2P/Agent qualification. Independent inspection confirms
the actual private namespace, ext4 `/dev/sdb1`, unchanged parent mounts and empty
temporary data, including the failed run.

The exact39-case Meta contract binary also passes on VM:
`meta-contract-20261001T073756-cab46e17`, including the unchanged expiry case.
Executable SHA256:
`16b3e0320d3218ea300a1826e0640b84a8af097d1c72cc5dbceb6717837659bd`;
archive SHA256:
`4bc1abf86d360fda57056ccef8dc88711dfc721ab51611b222484139c17786ea`.
Its generic runner scope wording was too broad; a separate correction identifies
the39 Meta/DFS cases and preserves the original result. Independent namespace,
mount and data cleanup inspection PASS. This VM result does not turn the failed
WSL project run into a success or qualify unexecuted suites.

Next: exact-generation Node refusal/drain proof, persisted pending commands and
cursor, actual Agent supervision/fencing, guarded normal detach and truthful
ACK production. Native admission, POSIX/P2P/mmap, complete lifecycle and paired
native-ext4 performance remain open. No PR or merge has occurred.

## Previous checkpoint: native lock Home authority and release outcomes (2026-10-01)

Source parent is `0fdedce`; exact tested dirty source is preserved in external
snapshot `native-lock-authority-candidate-20261001T064614`. The actual behavior
RED in `native-lock-authority-red.log` records28 PASS/4 FAIL: an old open
capability borrows recovered Home authority, an epoch-only target accepts the
retired fencing token, a blocking acquisition acknowledges after recovery, and
a failed normal owner unlock is reported as cancellation while the kernel lock
remains held. Recovery cases call the production `reconcile_on_startup` path;
the unlock case substitutes an actual same-object O_PATH descriptor in the test
coordinator, retains the original locked description, and checks an independent
native contender before and after retry. Cleanup precedes the assertions.

Native lock keys now include the admitted Home/holder sessions, nodes,
access_generation and fencing token as well as Root/epoch/file identity.
Local and authenticated Home peer routes use the Home grant. Old native-eligible
open capabilities are revalidated before choosing a target or cloning its
descriptor; post-wait validation fences the exact retired table. Fresh recovered
authority selects a new table. Ordinary OwnerFs keys remain epoch-based.
Normal release changes granted outcomes to Cancelled only for descriptions
whose kernel unlock succeeds; failed pins and granted outcomes survive for retry.

`native-lock-authority-green.log` records all32 native library cases PASS,
including all4 original failures. `native-lock-authority-lib.log` records385
all-feature library PASS/2 existing ignored. Strict workspace/all-target/all-feature
Clippy and fmt check PASS. `native-lock-authority-project.log` records exit0 for
the root-project all-feature test command and all25 executed suite summaries
PASS; ignored environment-dependent cases remain ignored. The earlier separate
workspace/all-ABI fuser failures and DFS clock diagnostics remain preserved.
No-feature, OwnerFs-only and DFS-only `cargo check --all-targets` also PASS.
No-feature/DFS-only variants retain the two existing descriptor-helper
dead-code warnings; strict per-variant lint has not been claimed.

Fresh VM replay `foundation-20261001T065423-3a76f7aa` records17 foundation PASS,
using the all-feature `ownerfs_native_linux-18984ae9bfa33684` binary SHA256
`1faa8b4e60512e38805343f415b5dbd911e0ec6d25333741f3cea63b9cf47870`.
Independent archive inspection verifies17 unchanged parent mountinfo pairs and
17 empty disposable-data directories. Archive SHA256:
`858bde97d3a7418f93004846d6a3ac398ddae9d46d9d62cc3714d149cc91282b`.
Seven stronger-directory diagnostics remain explicitly NOT_RUN.

The four new library regressions also PASS on actual VM ext4 `/dev/sdb1` under
an independently confirmed private namespace:
`lock-authority-20261001T065532-a2b3629a`. Exact all-feature library executable
SHA256 is `2dad2501d9990fb3ebfd13cc60326e686a3e4b8e68f5e99eff6f71ae278d431b`.
Independent inspection checks the3+1 test outcomes, unchanged parent mounts,
empty temporary data and ext4 source. Archive SHA256:
`ea61d191c81ade066fe41d88c032da24251d13138dc7458109c98ddd21bdb6e2`.
These are Home/kernel fixtures, not actual Node/Agent/network-P2P qualification.
The full feature goal and no-merge PR remain open.

## Previous checkpoint: native flock retirement and errno (2026-10-01)

Source parent is `54d6f59cc881d987afd26ebd57cd5e26e688f18d`; exact tested dirty
inputs are retained in external snapshot
`native-flock-retirement-candidate-20261001T062706`. This remains a foundation
slice. Node native admission, managed Agent fencing, applicable POSIX/network
P2P/mmap, full deletion/reclaim/switch/recovery and native-ext4 performance are
not complete; no PR has been created or merged.

Five new regressions cover all512 retired inode-table slots, exhausted session
history, blocked-waiter invalidation without recreated outcomes, errno conversion
through the public OwnerFs/FUSE mapping, and actual failed kernel unlock with
retained cleanup pins. Permanent invalidation rejects new locking and replay,
clears obsolete terminal/session history, and wakes blocked threads. Descriptions
whose unlock failed remain pinned and non-idle; session cleanup retries them
without creating new history. An actual valid O_PATH fd produces EBADF on
unlock; neither unsafe fd reuse nor a synthetic successful unlock was used.

Initial malformed-test compiler errors and a corrected cleanup-API assertion
are preserved separately. Actual behavior REDs:
`native-flock-retirement-red.log` reports both unreclaimable invalidation cases
and EIO substituted for EBADF; `native-flock-registry-red.log` proves all512
retired tables prevent a new inode from being admitted;
`native-flock-unlock-errno-red.log` reproduces the actual unlock errno loss.
The intermediate `native-flock-retirement-green.log` is an overall FAIL:
26 cases pass after retirement cleanup, while the separate errno case still
fails. The name is not a successful outcome; its exit record remains1.

The final `native-flock-retirement-lib.log` records381 all-feature library PASS
and2 existing ignored. The full root-project command
`cargo test --all-features --message-format=short` exits0 in
`native-flock-retirement-project.log`, including39 Meta tests and45 native
contract/event/journal/transaction tests. Ignored kernel/FUSE/RDMA/Redis cases
remain ignored. Per-suite accounting is saved alongside the log. The earlier
DFS expiry failure and realtime/monotonic observations remain unchanged evidence;
this successful run does not establish a clock fix. Earlier third-party fuser
full-workspace/all-ABI test failures also remain unqualified by a root-project
run. Strict workspace/all-target/all-feature Clippy and fmt check PASS.
`cargo check --all-targets` also exits0 for no features, OwnerFs-only and
DFS-only. No-feature and DFS-only checks emit the two existing OwnerFs-only
`try_clone_descriptor` dead-code warnings in localfs.rs; these runs are
compilation checks, not a claim of warning-free strict lint for every variant.

The source interface change is limited to an added `LockError::Kernel(errno)`
variant plus native OwnerFs conversion. The shared userspace lock model and
DFS do not produce the new variant or change their state transitions. Standard
flock errno cases EBADF/ENOLCK/EINTR/EINVAL/EAGAIN/EOPNOTSUPP/ENOMEM are checked
through existing stable domain codes and FUSE errno conversion. Kernel ENOSYS
uses the existing Node VFS unimplemented classification. Kernel syscall errors
are not mislabeled as poisoned mutex state. This does not introduce a general
raw-errno extension to the shared wire protocol or error catalog.

Current VM binary: default-feature `ownerfs_native_linux-79c664a798ade2c9`, SHA256
`24edd642eb2d8dd46684a54613c8bd57e15049bd840c6fa9249943498f15fae3`.
`foundation-20261001T062715-d249d32a` records17 classified private-namespace
foundation cases PASS, including native/Home flock arbitration and Home-grant
revoke/busy/normal detach. All17 parent mountinfo pairs match and all17
disposable-data directories are empty in independent raw-archive inspection.
Archive SHA256:
`bf46eb10063fc9b853dafe4439839f18cb477ef7402afbca5bea0a27228c262f`.
Seven stronger-directory diagnostics are explicitly NOT_RUN. These are in-process
Home/Meta fixtures, not actual production Node/network peer/Agent/performance
qualification.

Next integration prerequisites are recorded in the plan: exact native lock
Home-authority revalidation and failed-release terminal outcomes, plus Node
consumption of a bounded current-authority control feed. The existing Meta
WatchRootCommands RPC returns one finite filtered batch; an empty/ended stream
is not by itself an ongoing authority lease. Compaction and cursor advancement
must be addressed before relying on it for native Agent admission/fencing.

## Previous checkpoint: revoked Home authority and normal teardown (2026-10-01)

Source parent is `034a188dfc40495d941f6756bc6d616344248092`; the tested dirty
inputs are preserved in external snapshot
`native-home-authority-candidate-20261001T060529`. The current default-feature
VM binary is `ownerfs_native_linux-79c664a798ade2c9`, SHA256
`78b2f290b70cb26a75a863f0878a20fa2eb4f7af4c328d046bce6087a583d504`.
This checkpoint remains a foundation slice, not production feature completion.

- Default-feature library:374 PASS/1 existing ignored. All-feature library:
  376 PASS/2 existing ignored. Strict workspace/all-target/all-feature Clippy
  and final fmt check PASS. The separate default-feature native controller,
  event, journal and transaction suites record45 PASS in
  `native-home-anchor-contracts.log`. Whole-project/workspace failures recorded below
  remain failures; this targeted verification does not qualify their remainder.
- Real VM paired Home-authority case:
  `directory-20261001T060722-20345151`, both mode0 ordinary-policy rejection and
  mode1 native lifecycle PASS. Revoke rejects old FUSE file reads and new
  native admission. With a held native fd, normal unmount returns EBUSY and
  the physical mount stays present. After that fd closes, normal detach succeeds,
  backing bytes remain unchanged, and released metadata anchors no longer
  expose the revoked Root. Native fds themselves are not revoked by RootGrant;
  actual Agent process fencing still needs independent implementation/validation.
- All17 classified foundation cases PASS, each in its own private VM namespace:
  `foundation-20261001T060800-c10c83fd`. Seven historical stronger-directory
  diagnostics are explicitly NOT_RUN, not relabeled as passes. Independent
  raw-archive inspection verifies17 unchanged parent mountinfo pairs and17
  empty disposable-data directories. Archive SHA256:
  `d47f76909efcdaf13565c75ffd35bf6784f3bd1cc597137f1538f9d33eacb921`.

The new source-bound preparation API uses the opaque permit's actual source
fd and retains its metadata anchor in the Linux prepared record. Positive
Root lookup/getattr metadata remains available only while management retains
that anchor. It does not confer data rights, peer admission, a native lease,
or permission to reclaim backing. Native-eligible local file operations capture
Home authority on open and re-admit on read/write/fd metadata/truncate, comparing
Root/epoch/Home/holder sessions, generation and fencing token. The admission
remains in flight through I/O. Previously accepted dirty bytes may still be
synced during retirement, and handles can be released. Ordinary OwnerFs/DFS
behavior stays on its existing policy.

Behavior REDs are preserved: `directory-20261001T053718-f8a77ce3` and
`directory-20261001T054658-bb3a8eb6` show Root revocation followed by FUSE
revalidation losing the submount implicitly; held native references survive.
The first anchor fix, `directory-20261001T055207-9be8dac9`, retains the busy
mount and allows normal teardown but FAILS because old FUSE reads still work.
`native-home-anchor-data-red.log` independently reproduces that read denial
assertion before the fix. Earlier malformed-test compiler failures are retained
separately and are not behavior REDs. Three new library regressions cover root
metadata anchor release, rejected read/write/metadata/truncate with successful
retirement flush/fsync/release, and retired authority identity rejection.

Production Node/Agent namespace readiness, POSIX-lock ownership decision and
implementation, actual network P2P/mmap, full deletion/reclaim/switch/fencing,
boot/namespace/daemon recovery and paired native-ext4 performance remain open.
No PR has been created or merged at this checkpoint.

## Accepted native contract (2026-10-01)

The user accepted the [native profile and concrete cases](../docs/architecture/ownerfs-native-access.md): management verifies the export in the final Agent namespace before starting the Agent; cross-path file visibility is close-to-open; retained FUSE directory references need not immediately track native rename/delete. Native plain close is a visibility endpoint, while explicit applicable sync establishes durability. Ordinary FUSE-only/DFS defaults remain unchanged. Earlier strong directory/cache diagnostics retain their actual assertions and results; they are boundary evidence, not product acceptance passes. Production admission, applicable cache/locks/lifecycle and performance work remain incomplete.

The candidate and outcomes below describe recorded historical checkpoints, not qualification against the revised profile.

## Historical candidate before the accepted profile

- Current base: `78245771167643d5883491052e7cebcaba8c3be2` (original base `6bcabe8f30040bc6cc3b518bd271e7e2461e1e1d`); branch `feat/ownerfs-native-bind`. Feature worktree is isolated from the canonical checkout and the other machine's main branch.
- Rust1.95 x86_64 Linux. WSL6.6 builds/tests the controller and journal; actual mount backend is run in Linux6.8 VM A on `/dev/sdb1`, ext4 UUID `6fa5e173-b766-4c27-872b-8f40e91bed27`.
- Current VM test binary SHA256 `cbce155edfffe9e758004b0708a312509b1176a3441999695d32e4c5738de02c` (19 cases; read-only directory repair and deterministic concurrency counterprobes; full semantics remains RED).
- Private mount namespaces; test data confined to uniquely created directories. Earlier successful probes checked parent namespace mountinfo byte-for-byte. The ordinary RED driver exits before its cleanup markers; the paired portable RED replay independently verifies parent mounts and disposable-data cleanup. Most covered test directories are disposable ext4 directories. One test now uses the real OwnerFs FUSE adapter and RootManager with an in-process Meta fixture; it does not run production Node/P2P or enable native policy.

## Historical outcomes before the accepted profile

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

## Native cache profile and close-to-open regression

`OwnerFs::new_native_eligible` now selects direct-I/O ordinary file replies and
zero entry/attribute TTL from construction, for every root in that OwnerFs
instance. It has no runtime setter, authority grant, automatic mount or native
ready ACK. Existing ordinary instances retain their private/shared policies.
Authenticated peer grant validation remains in place; native-eligible instances
need no first-share cache invalidation because they never issued private replies.

Two unit tests verify new-root and recovered-root policy plus absence of hot
conversion. A behavior RED observed native LOOKUP entry TTL still1s with attr0;
the final lookup policy fixes entry TTL to0 while preserving ordinary shared
entry TTL1s. Initial missing-constructor diagnostics and an earlier compiler
renderer/test-context failure are retained separately from that behavior RED.
Final library:355 PASS/2 existing ignored; strict all-target/all-feature Clippy,
fmt, integration build, shell syntax and diff whitespace checks pass.

Final21-case binary SHA256 `325ba24bf3f55a324e8a5cad64678b355e368494120c3a6520e6a25dcafd5fd9`; run
`directory-20261001T040518-8ed09be5` selects exactly
`privileged_native_close_to_open_profile` (1 executed,20 filtered,none ignored).
Native profile PASS; ordinary-private diagnostic control FAIL. The latter first
opens OBJECT-A after growing from a previously empty file and reads zero bytes:
this is a fresh-reader close-to-open counterexample, not an assertion that an
already-open reader must refresh instantly. Both modes independently leave
parent mountinfo unchanged and disposable data empty. Collection exit0 does
not relabel the control test exit101.

The fixture uses real OwnerFs/FUSE, a journaled mount controller, and two Home
FUSE sessions over one authoritative backing. It checks growth/shrink/empty
data and lengths, same-name replacement/new open, retained original file fd,
stable-root directory reopen after native move, missing name after unlink and
unmount-to-FUSE fallback. The management fixture verifies the native export
before spawning a Python Agent; its cwd device/inode and inherited mount
namespace match the native source. This is not production Node orchestration,
network P2P, shared-lock/mmap qualification or a performance measurement.

The portable probe accepts an optional EXACT_TEST_NAME and records full
discovery plus selection. Without it the original full diagnostic suite still
runs; none of the historical stronger directory assertions was removed.
External inputs: `native-cache-profile-inputs.json`, discovery and
`vm-native-cto-final/`. Reproduce with eligible cache enabled and the recorded
binary: `AFS_NATIVE_ELIGIBLE_CACHE=1 ownerfs_native_mount.sh BINARY FRESH_EXT4_DIR SHA privileged_native_close_to_open_profile`.

## Current-policy verification before first or duplicate native READY

A manager defect returned NativeActive for a matching mount identity even after
that mount was remounted read-only against an expected read/write policy. Actual
VM RED `directory-20261001T040836-28ba457b` reproduces this twice, preserving mount
ID/source identity, backing contents and independently verified cleanup. Two
controller RED tests also prove first and duplicate ACKs lacked policy checks.

Every activation ACK now calls the backend's live effective-policy verification,
after matching its physical identity. A first post-attach policy error preserves
the owned claim and observation in Recovering; no false FUSE-only response or
backing deletion follows. Duplicate policy failure likewise reports an error
without stacking/removing the mount. After restoring the trusted flags, verified
reconciliation and normal teardown succeed. The VM wrapper delegates real policy
checks even in helper-crash tests.

45 targeted native tests pass:18 controller,5 event,10 journal,12 transaction.
Strict all-target/all-feature Clippy, fmt and integration build pass. Final22-case
binary SHA256 `98866ee20e8ebd5f3d5db4e9ef2810cb4d2affbc3a62b64a1c01dca08ebbd2ff`; GREEN
`directory-20261001T041340-d2d10563` repeats the exact ready-policy case twice.
No claims of Node/Agent authority admission follow from controller readiness.

`foundation-20261001T041347-628e067e` replays15 mount/cache/readiness foundation
cases individually in fresh private VM namespaces, all PASS, none ignored. Each
portable case checks unchanged parent mountinfo and empty disposable data. The
tracked profile manifest partitions all22 discovered tests into15 selected
foundation cases and7 retained historical strong-directory diagnostics; any
missing/new/unclassified test rejects replay. The latter are explicitly NOT_RUN
in this foundation replay, never relabeled as passes. The original unfiltered
probe still executes them and retains their original assertions.

`ownerfs_native_profile.py` is a foundation regression driver, not full native
product acceptance. It does not cover production Node bootstrap, remote network
P2P, kernel-visible locks, supported mmap, authority/Agent fencing, complete
recovery or performance. Raw logs/manifest/results are preserved in external
`vm-native-foundation/raw-evidence.tar.gz`; RED/GREEN logs and exact source hashes
are in `vm-native-ready-*/` and `native-readiness-inputs.json`.

## Home kernel flock arbitration (2026-10-01)

Two source behavior REDs prove that the old userspace table grants a FUSE/Home
lock while a native exclusive flock is held, and vice versa. The initial
unsafe-code lint error is preserved separately, not counted as a behavior RED.
Native-eligible Home now duplicates the actual open backing description and
uses Linux flock through Rust's safe file-lock interface. No pathname reopening
or generic storage-trait/DFS lock change is involved. POSIX byte-range locking
still uses the existing model and is not qualified for native admission.

The coordinator bounds descriptions to1024, waiters to64, terminal/replay
identities to128, and closed scopes to128 per inode. Outer registry limits remain
512 inode tables/1024 routed waiters/4096 ingress scopes. Resource exhaustion
fails admission rather than evicting replay fences. Cancellation/grant and
cleanup share one mutex; the open slot stays locked through each NB kernel
attempt. Native external unlock is detected by real kernel attempts scheduled
at most every10ms during a blocked wait, never by a sleep-based grant. Pins are
explicitly unlocked on owner/session/final-handle/root cleanup; failed unlocks
retain pins for retry. A separate RED detected that released, unacknowledged
grants still reported Granted; cleanup now changes them to Cancelled.

10 source regressions PASS: bidirectional arbitration, cancelled waiting,
native-unlock wake, session/final-file fencing, released-grant outcome, shared
upgrade/old unlinked file identity, Root revocation and authenticated Home peer
session cleanup. These are Linux source/backend tests, not network P2P evidence.

Actual VM pair `directory-20261001T044050-cc08f387`, binary SHA256
`7005881f6278a0352b466b7c26c5866a9660733582e5b4d74526e96f2840d3e5`, runs the exact
`privileged_native_flock_kernel_arbitration` case in two fresh private namespaces
on kernel6.8.0-142/ext4. Mode0 ordinary-table control FAILS as expected; mode1
native-eligible PASS. Checks cover native-to-FUSE, FUSE-to-native, independent
FUSE descriptions, shared locks, Linux failed-NB-upgrade semantics and retained
object locks across native rename/replacement/unlink. Both normal teardown paths
join FUSE sessions before asserting behavior; unchanged parent mounts and empty
disposable data are independently verified even for the failing control.

`foundation-20261001T044252-91f41aa0` replays16 foundation cases with that same
binary, all PASS. The manifest classifies all23 discovered cases into16 selected
foundation cases and7 unchanged historical stronger-directory diagnostics,
explicitly NOT_RUN in this lane. Its scope now includes local kernel flock,
not native POSIX locks, network P2P, mmap, Node/Agent admission, full lifecycle,
performance or release acceptance. Full source snapshots/patch/hashes are in
external `native-flock-first-candidate/`; raw pair and foundation archives are
in `vm-native-flock-pair/` and `vm-native-flock-foundation/`.

Strict workspace/all-target/all-feature Clippy and fmt PASS. The full
`cargo test --workspace --all-features -- --test-threads=1` command FAILS in
unchanged vendored fuser tests: `ll::request::tests::init` returns InsufficientData
for its 7.8 INIT fixture under the larger all-feature ABI input layout;
`reply::test::reply_create` passes flags0xcc, including FOPEN_PASSTHROUGH0x80,
to an API that explicitly rejects that flag under abi-7-40, then its AssertSender
panics again during reply cleanup and aborts. Each reproduces with standalone
`cargo test -p fuser --all-features --lib TEST -- --exact --test-threads=1`,
which does not compile or execute OwnerFs. Git confirms fuser and Cargo manifests
are unchanged from the slice base. Preserve the complete failing workspace log
and both isolated logs; do not classify the workspace command as PASS or remove
these failures. afs/library365 PASS/2 ignored and other executed suites preceding
the abort pass, but the aborted remainder/doc tests are not thereby verified.

The normal project command `cargo test --all-features -- --test-threads=1`
subsequently PASSes with its configured fuser ABI feature set:365 afs/library
PASS/2 existing ignored, project integration suites and doc-test traversal
complete. Environment-dependent ignored cases retain their names and reasons
in `native-flock-root-project.log`; this does not qualify them or turn the
separate failing workspace/all-feature fuser run into a pass.

## Home authority admission bridge (2026-10-01)

An opaque permit now binds its issuing OwnerFs instance, current Home grant,
namespace and confined backing directory object. Issuance requires prior
construction-time native cache eligibility and Home Lookup/Read/Write. The
actual directory descriptor is pinned; current grant fencing/session/rights,
configured directory and reopened object identity are revalidated. Retaining
this permit does not retain an in-flight RootUse or grant a native-access lease.

Guarded physical activation requires the registered Root/epoch/Home/source
match and current authority before and after attachment. Lost authority uses
normal owned-export rollback; EBUSY retains the mount and Draining claim.
This is not production descriptor preparation, Node admission, Agent readiness,
continuous authority fencing or reclamation. Those integrations remain open.

The initial `native-home-authority-api-red.log` is an API-missing compiler RED,
not a behavior failure. A subsequent actual behavior RED in
`native-home-authority-foreign-owner-red.log` showed that a foreign OwnerFs
caller could trigger cleanup of another instance's valid export. The fixed
origin check now runs before any mutation, including cleanup. All8 final
source regressions PASS: ordinary cache-policy rejection, namespace/name/current
grant checks, source replacement rejection, stale grant and wrong-spec no-bind,
post-bind authority loss rollback, foreign-instance no-detach, and busy rollback
claim retention followed by normal teardown. These use a mount-backend probe,
not a real kernel Home-grant attach or an Agent supervisor.

`native-home-authority-project.log` records373 library PASS/2 existing ignored,
including all8 new cases, then an overall FAIL in the unchanged DFS test
`dfs_renewal_cannot_resurrect_expired_or_reassigned_authority` at
`tests/meta_contract.rs:186`. The exact test independently FAILS in
`native-home-authority-dfs-expiry-isolated.log`. Git verifies that the Meta test,
DFS implementation and Cargo manifests have no changes in this slice.

The test waits1100ms using Tokio's timer and tests expiry using SystemTime. An
initial read-only WSL observation measured602ms realtime advancement against
1100ms monotonic advancement. `native-home-authority-clock.log` preserves five
further intervals, including realtime jumps of+8668ms and-6950ms while each
monotonic interval remained about1100ms. These observations support a clock
domain explanation for the expiry assumption; the source of the clock steps
has not been established. No system clock/service or Meta assertion was changed.
Keep the original failed run and do not classify its unexecuted remainder/doc
tests as passing or remove the expiry case.

The separate `native-home-authority-contracts.log` records45 native contract,
event, journal and transaction tests PASS; strict workspace/all-target/all-feature
Clippy PASS in `native-home-authority-clippy.log`, with fmt PASS. Current source
snapshots and hashes are saved in external `native-home-authority-candidate-*`.
No new VM admission or performance claim follows; the prior16 VM foundation
cases retain their eafdabf-slice inputs and binary identity above. Production
Node/Agent, applicable POSIX/P2P/mmap, lifecycle and performance gates remain open.
