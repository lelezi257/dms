# OwnerFs native bind implementation plan

> For agentic workers: use superpowers:executing-plans inline. Preserve the existing RFC and completed probes. Use failing Linux regressions before implementation. Submit a PR for review; do not merge it.

**Goal:** Complete the same-path native OwnerFs feature with correct applicable filesystem/lifecycle semantics and measured native-ext4-level local performance. A mount-only manager is an intermediate result, not completion.

**Architecture:** OwnerFs remains the authority and file/P2P backend. A dedicated native manager controls verified workspace exports in the managed namespace. FUSE is a consistent fallback and transition path to the same ext4 object. Existing OwnerFs/DFS state and RPC boundaries remain separate.

**Tech stack:** Rust1.95, existing libc/serde/fuser, Linux guest ext4, mount identities and descriptor-based mount API; independent Home/peer Node mounts. No new third-party dependency unless a demonstrated gap requires it.

**Spec:** `docs/rfcs/0001-ownerfs-native-bind-mount.md`, user-approved complete-semantics/native-ext4 goals, GitHub issue #42. Original baseline main `6bcabe8f30040bc6cc3b518bd271e7e2461e1e1d`; isolated feature rebased onto `78245771167643d5883491052e7cebcaba8c3be2`.

## Global constraints

- Retain completed probe evidence. Rerun only affected assumptions and product regressions; do not count historical failures as fixes.
- Preserve `/ownerfs/<workspace>`, one backing directory, fixed Home and managed Agent lifecycle. No copy/symlink substitution.
- Never wait for same-path bind before the FUSE mkdir reply. Old fd/dirfd/cwd do not become native merely because a mount completed.
- Implement the user-accepted native profile in `docs/architecture/ownerfs-native-access.md`: cross-path close-to-open, retained file-object identity and explicit old-FUSE-directory boundaries. Preserve ordinary FUSE-only/DFS defaults and all applicable lock/permission/mmap/lifecycle gates.
- Match root/epoch/Home session/namespace and physical directory/mount identities. Unrelated mounts/processes/data are never cleanup targets.
- Default configuration preserves existing behavior; enable native explicitly. No hot conversion of existing cached/mapped references.
- Failure/busy/unknown states stay visible. Lazy detach is not access revocation or permission to delete/reuse data.
- VM ext4 holds tested data. WSL is build/source regression only. x86 feature evidence does not qualify original ARM64 formal release or MooseFS/3FS baselines.
- Pair native and ext4 workloads on the same VM/volume with identical data, flags, sync policy and host-load accounting; report every repetition and uncertainty, not a selected fastest result or artificial tolerance.
- Native profile plain close confirms visibility, not durability; explicit applicable file/parent-directory sync establishes recovery watermarks. User accepted this visibility/durability separation. Ordinary FUSE-only/DFS close policies remain unchanged; measurements compare identical visible/durable endpoints.

## Review focus

1. An old asynchronous operation arrives after cancellation or same-name/epoch change: reject it without touching the newer root.
2. Target has stacked/foreign mount, symlink replacement or wrong namespace: refuse mount/unmount; never claim native readiness from path text alone.
3. Native write followed by successful close and a fresh current-path FUSE open must not return stale data/length. Retained file fd identity must not be rebound by rename/unlink/replacement; already-open cross-path reader freshness and retained directory references follow the accepted native profile. Verify supported mmap separately.
4. Native fcntl/flock competes with local/remote FUSE; close, cancellation and dead peers: use the Home kernel inode and retain precise owner/waiter lifetime, including same-process native/old-FUSE lock conversion and unlock.
5. Mount/fencing/persistence failure, retained cwd/dirfd/mmap, Node death and restart: retain recoverable identity/state and forbid premature deletion/reuse.

## Task 1: mount identity and lifecycle controller

Files: create `src/node/vfs/ownerfs/native.rs`, focused `native/mountinfo.rs`, `native/manager.rs` and their unit tests; add only the module declaration in `ownerfs.rs`.

Interfaces: `NamespaceIdentity { device, inode }`, `DirectoryIdentity { device, inode }`, `WorkspaceIdentity { root_id, epoch, home_node_id, home_session_id, namespace }`, `MountIdentity { mount_id, unique_mount_id, namespace, source, covered_target }`; `MountInfo::parse(&[u8]) -> io::Result<Vec<MountInfo>>`; `MountBackend::{inspect, bind, attached_claim, unmount}` with structured identities; `NativeMountManager::{register, activate, quiesce, detach, status}`.

- [x] Write mountinfo tests: octal-escaped path, malformed/missing separator, invalid IDs/escapes, two stacked entries retained independently. Expected first run: module/API missing.
- [x] Implement strict parsing without assuming line order identifies the top mount; live lookup must use the kernel's actual mount identity.
- [x] Write controller tests: duplicate activation invokes bind once; foreign mount unchanged; namespace/source/target mismatch refused; bind failure leaves FUSE_ONLY; stale epoch/session command rejected; failed/busy umount remains DRAINING; identity mismatch cannot detach; re-registration forbidden before detachment.
- [x] Implement per-root serialization, desired/observed state and operation identity with bounded registry; errors preserve state and original errno.
- [x] Run targeted tests and feature compilation. Commit the tested slice with scope limitations explicit.


## Task 2: Linux mount backend, journal and recovery

Files: create focused `native/linux.rs`, `native/journal.rs`, VM integration tests/driver under `tests/ownerfs_native_*` and `development/acceptance/probes/ownerfs_native_*`.

Interfaces: `LinuxMountBackend` consumes configured source/target directory descriptors and trusted prepared workspace identity. Use `open_tree/move_mount` plus selected mount flags; `MountJournal` persists versioned desired/operation identity and fsyncs its directory. Actual mount IDs/namespace are reobserved, never restored as current facts.

- [x] Add RED tests for exact-source same-path mount, mount policy, duplicate retry, foreign/stacked mount, target replacement, wrong namespace, retained fd/cwd/mmap busy umount, and syscall failure preserving data. Backend/controller failures and positive kernel controls are retained separately; this does not prove Node/Agent/P2P fencing.
- [x] Implement fd-confined Linux calls, mountinfo/statx identity confirmation, structured diagnostics and durable journal.
- [x] Add crash-point recovery tests before/after kernel mount/journal ACK. Only owned, identity-matched exports may be recovered or removed. Scope: helper process exit in the same surviving private namespace; Node/FUSE death remains Task5.
- [x] Run in a disposable private mount namespace in the independent VM; retain commands, identities, errno, contents and cleanup proof.

## Task 3: consistent OwnerFs transition/fallback and opt-in integration

Files: modify narrow hooks in `src/config.rs`, `src/node.rs`, `src/node/fuse.rs`, `ownerfs.rs` and root/catalog only where needed; add OwnerFs-specific policy/cache regression tests.

- [ ] Add RED tests for default-off behavior, invalid configuration, new eligible root policy from creation, immediate mkdir→create/read, bind failure and native→FUSE fallback.
- [ ] Configure the independent manager/worker after the mkdir reply; publish native readiness only after trusted authority, mount identity and policy checks in the final Agent namespace. Start/expose the Agent only after readiness; failure must not emit a ready ACK. Do not hold authority or FUSE locks while executing mount.
- [ ] Validate close-to-open after writer close and cached-reader close/reopen, file-object identity across replacement/unlink, stable-root fresh directory lookup and the actual ready-before-Agent workflow. Preserve historical already-open-reader and old-FUSE-directory diagnostics as boundary evidence. Validate supported mappings; unresolved applicable semantics still gate native admission.
- [ ] Verify real local-native/old-local-FUSE/remote-FUSE/P2P against the same Home objects, plus simultaneous DFS mount isolation.

## Task 4: kernel-visible locks and complete supported file semantics

Files: new OwnerFs-native lock mechanism where possible; narrow integration in OwnerFs and peer contracts, avoiding shared DFS lock behavior changes. Extend existing metadata interfaces only for proven missing operations.

- [ ] Add RED actual-native-versus-FUSE/peer fcntl/flock tests with native/native controls; cover range replacement, read/write locks, fork/dup/close, cancel before/after grant, long waits and peer cleanup, plus same-process native/old-local-FUSE owner conversion and unlock.
- [ ] Implement Home kernel-inode arbitration while preserving POSIX process ownership, flock open-description ownership, waiter identity, errno and cancellation. A separate user-space lock table alone cannot pass this task.
- [ ] Add RED native/transition/remote differential operations: append/EXCL, truncate, hardlink/symlink, chmod/chown/umask/SGID/sticky, xattr/time, rename/unlink/replacement with old handles, directory enumeration and sync.
- [ ] Implement required gaps and verify mmap MAP_SHARED/MAP_PRIVATE/msync/fsync/close behavior under the accepted contract. Do not add unsupported exclusions after observing failure.
- [ ] Run OwnerFs applicable POSIX/FSx/random suites and shared FUSE/DFS regressions; preserve all results and discovery accounting.

## Task 5: managed delete/reclaim/switch, authority fencing and process lifecycle

- [ ] Add RED tests for active writers/peer requests, retained fd/dirfd/cwd/mmap, duplicate delete, same-name new epoch, delayed helper, Home/session loss, Node/helper crash and restart.
- [ ] Implement explicit managed lifecycle entrypoints: stop admission; drain/stop managed users and fence remote handles; normal-unmount all owned exports; only then alter authority/backing. Never treat lazy detach as drainage.
- [ ] Derive drainage from actual managed process/reference state and kernel busy checks. Validate PID/start identity before signaling; no global process discovery or guessed PIDs.
- [ ] Reconcile residual exports and dead FUSE/socket state before admission; preserve unknown/foreign state for diagnosis instead of deleting it.

## Task 6: native ext4 performance, package and reviewable delivery

- [ ] Create paired workloads for sequential/random data read/write, explicit sync variants and small-file create/stat/readdir/rename/unlink. Inputs and durability flags are identical for candidate and ext4; namespace creation/mount setup timing is separate and fully reported.
- [ ] Pin source/package/PID/SHA/mount/volume/kernel/config identity; verify zero Home FUSE requests for warm native operations with an old-FUSE positive control. No correctness or performance PASS from a detached/unidentified process.
- [ ] Run repeated paired VM measurements with fixed cache/data/resources and host load; report full samples, ratios and uncertainty. A stable slowdown remains an unresolved goal item.
- [ ] Run fmt, strict Clippy, relevant Linux unit/interface/FUSE tests and none/ownerfs/dfs feature matrix; build/install candidate with preserved evidence and explicit enable/reject behavior.
- [ ] Update RFC/status/implementation/evidence index. Perform final fresh review, address material findings, push isolated branch and create PR linking #42. Attach PR to this chat. Do not merge, enable auto-merge or alter the other machine's main branch.

## Execution ledger

Entries below preserve the contract and next steps used at each historical checkpoint. The accepted native profile revision at the end supersedes earlier pending durability decisions and requirements for instantaneous old-FUSE-directory repair; those historical next-step statements are not current implementation instructions.

- Pre-flight: controller identities feed kernel/journal; accepted durability/cache/lock contracts gate integration; all functional evidence gates performance; only tested final candidate gates PR claims.
- Ruling: native App work uses the already-approved managed lifecycle/fixed Home RFC, while complete applicable semantics remain the goal; mount-only results cannot mark the goal complete.
- Ruling: Codex native worktree tool cannot operate the non-Git Windows outer directory. Git fallback worktree at `/home/lzc/workspace/dms/worktrees/ownerfs-native-bind`, branch `feat/ownerfs-native-bind`; canonical checkout and another machine's branch stay untouched.
- Ruling: preserve existing v25 build/package and VM lane. Feature builds use a separate target/evidence path; test runs use private, identified namespaces/lane prefixes and do not stop unrelated processes.
- 2026-10-01: issue #42 created. Native durability clarification pending; tasks1/2 can progress independently. Existing root grants are authority, physical paths are not authority.

- Ruling: use byte-based mountinfo parsing, preserving non-UTF-8 names; live identity comes from statx, not mountinfo line order. Cost: consumers must handle OsStr paths.
- Ruling: ownership includes Linux6.8 STATX_MNT_ID_UNIQUE plus persisted boot/namespace/source/target identity. Recyclable mountinfo IDs are diagnostic only. Cost: older kernels reject native preparation (WSL6.6 is build/regression only); the feature defaults off.
- Ruling: a bind error requires the backend's actual attach claim before a matching observation may be owned. Cost: uncertain/foreign state cannot automatically clean itself up.
- Ruling: a write/rename/directory-sync error poisons the journal until reopen and reconciliation, preserving orphan temp evidence. Cost: availability is sacrificed after an uncertain persistence result; orphan maintenance still needs explicit recovery implementation.
- 2026-10-01: controller 16 + journal8 tests passed in WSL. VM A Linux6.8/ext4 independently passed5 backend tests, including same-path identity, busy retry, nosuid/nodev/ro/noexec, foreign-manager refusal, source pinning and target substitution. These cover mount primitives on disposable ext4 directories, not OwnerFs FUSE integration.
- 2026-10-01: post-rename directory-fsync failure and matching-foreign-mount-after-bind-failure regressions were observed failing before fixes. All RED logs remain external. Rust1.95 diagnostic rendering itself panicked on one unresolved-import RED; split imports/short diagnostics produced the intended unresolved-import error.
- 2026-10-01: the first VM driver passed the mount test but failed a stale requirement that old environment mounts still existed. Inspection found no old Node/FUSE lane running; no stop operation was issued. Corrected driver compares actual parent mountinfo before/after and succeeded. Failed raw log is retained.
- Remaining after the first primitive checkpoint: controller transactions/recovery were then unwired. The later transaction checkpoint below supersedes that limitation only for the tested surviving-namespace helper crash scope; full feature completion remains pending.

- 2026-10-01 checkpoint: fmt and all-targets/all-features strict Clippy passed; unchanged-library serial regression258 PASS/2 ignored. Portable VM probe reproduced5 PASS, no parent mount changes. Evidence summary `native-bind-evidence.md`. Feature completion and PR still pending.

- 2026-10-01 transaction checkpoint: 32 controller/journal/transaction tests passed; all8 actual VM backend tests passed, including helper exits before attach, after attach and after normal umount. Same-namespace restart requires current trusted spec, exact unique mount claim and effective flags before NativeActive. No Node admission, Agent drainage, boot/namespace-loss recovery or performance claim.
- Ruling: `bind_journaled` synchronously persists the allocated exclusive clone ID before kernel attachment; errors never fall back to an unjournaled bind. `Unmounting` persists normal-unmount intent before removal. Cost: persistence failures close mutation until reopen/reconciliation.
- Ruling: journal schema2 requires retired Home session history. Version1 prototype state is preserved and rejected. Cost: old development journals require explicit inspection; no deployed Node native format exists to migrate automatically.
- Ruling: nonrecursive parent clone reveals the covered target for recovery without touching a live export; current effective flags independently gate readiness. Cost: same surviving namespace/parent is required for this path; source-authority and Node/FUSE death recovery remain separate requirements.
- RED/GREEN: foreign target self-bind and changed recovery policy both reproduced on VM before fixes. Current exact binary SHA and raw exit/log evidence are indexed in `native-bind-evidence.md` and the external transaction input manifest.

- 2026-10-01 reference checkpoint: actual VM11/11 and portable replay11/11 passed. Real OwnerFs mkdir/FD attachment/re-preparation/fallback preserves same backing; native dirfd/cwd/shared-VMA/private-VMA each prevents normal unmount until its exact actor exits. Tests use an in-process Meta fixture, not production Node or remote P2P.
- Ruling: `release_prepared` releases only absent, unclaimed, identity-matched pins; it never unmounts, revokes authority or deletes backing. Cost: unresolved externally removed claims remain blocked until verified recovery or supervisory teardown, and Node lifecycle still must connect this API.
- Pending question: may native mode require capability-detected Linux6.9+ FUSE passthrough in a separate feature environment while older kernels retain ordinary FUSE? The existing6.8/formal lane is untouched. No passthrough implementation or semantics PASS follows from source support alone.
- Remaining: Node opt-in/event/helper/supervisor, cache/locks/P2P/permissions integration, orphan maintenance, daemon/namespace/boot-loss recovery, authority/Agent fencing and all performance work. Durability and proposed platform contract decisions remain pending.

- 2026-10-01 maintenance checkpoint:38 unit/transaction passes and12 actual VM/portable passes. Bounded same-identity orphan cleanup has fresh export checks and an unlink/directory-fsync uncertainty barrier. Foreign/future/unknown files remain evidence; no source data is deleted. Node/boot-loss lifecycle integration is still pending.
- RED/GREEN: another root's journal rename failure left an existing root's duplicate registration returning stale success. Duplicate ACK now checks global journal health and rejects EIO until reopen; the reproduction and recovery control are retained.
- Design rejection observed on VM: native same-process POSIX downgrade succeeds, but a helper inheriting its fd and supplying its PID or using an OFD lock gets EAGAIN. Do not implement the naive proxy and claim full owner semantics. Passthrough I/O/mmap is not evidence that POSIX/flock delegation is solved. The tracked counterprobe and required owner case remain Task4 inputs.

## Post-reply runtime event slice

Before Node activation, wire a bounded workspace-created hint channel into the
real OwnerFs FUSE mkdir reply. Publish only after reply and only for a direct
child of the OwnerFs root. Preserve raw byte names and opaque backend inode;
neither is an authority grant or a persistent identity. A worker must reacquire
trusted current authority and physical prepared identities before manager calls.

The producer never waits for the consumer. Overflow marks an explicit rescan
requirement; the consumer clears it before scanning the current authoritative
inventory, and concurrent overflow during that scan requests another pass.
Disconnect remains observable and does not enable a native path. Install once
before mount in the future opt-in bootstrap; default constructors have no sink.
No native eligibility/cache policy, manager worker, or native admission follows
from this notification slice alone.

RED tests: invalid capacity, non-UTF-8 byte-preserving payload, full-queue
rescan/rearm, disconnected producer, duplicate sink installation, and actual
VM FUSE mkdir/nested mkdir/full-queue operation without an active consumer.
Then implement events and narrow OwnerFs/FUSE hooks; run targeted/all-target
checks and execute real kernel FUSE cases in the independent private VM lane.

- 2026-10-01 event checkpoint:5 channel tests plus38 controller/journal tests pass; real VM and portable13/13 pass. The FUSE adapter sends root-created hints after reply, without waiting for the consumer; nested mkdir is excluded and full queue requests rescan. Existing library258/2-ignored rerun, fmt and strict Clippy pass. Independent trusted-authority worker/native eligibility/Node activation remain pending.
- Main coordination: a read-only fetch found origin/main at7824577, including a joined FUSE cleanup result, callback drain and OwnerFs surviving-hardlink alias fix. Preserve the current event checkpoint and evidence before rebasing only this isolated feature branch onto that committed main. Never transfer old evidence to the new candidate without scoped revalidation. The canonical checkout and the other machine's branch remain untouched.

- 2026-10-01 main alignment completed: only feature branch rebased; backup retains the old event tip. Scoped352 library/43 native and13 VM/portable cases passed before the new counterexample. No changes to canonical main or the other machine's branch.
- 2026-10-01 directory counterexample: an old FUSE dirfd/cwd does not automatically follow native rename; first access is ENOENT/old parent. Forced lookup can relocate the kernel alias. A RED library test proved fresh lookup still left descendant paths stale; subtree reconciliation fixes that explicit-refresh defect. Current353 library/43 native checks pass, but full VM14 suite remains13 PASS/1 FAIL. Automatic source-object/alias repair remains required; retain the first-observation assertions.
- Next: probe an automatic repair mechanism with actual old directory references and concurrent native mutation, preserving source-object identity and kernel parent semantics. Do not substitute a user relookup, weaker assertion, inotify timing assumption or copied-path fallback. Keep Node activation gated while cache/locks/durability and authority lifecycle contracts are unresolved.

- 2026-10-01 callback/cache counterprobe: the full14-case suite remains13 PASS/1 FAIL in ordinary private and existing peer-shared controls, using one binary and fresh VM namespaces. Parent-only old dirfd/cwd emits no moving-directory GETATTR while private TTL is live and reads stale LEFT; shared zero-TTL emits two GETATTRs and returns ENOENT. After1.2s both emit three GETATTRs and return ENOENT. Forced alias lookup fixes the controls only; original first-access assertions remain RED. Both portable failing suites have independently verified clean parent mounts and disposable data. No production policy changed.
- Next directory mechanism constraints: independently validated bounded source-object references; a pre-native cache barrier; off-receive-thread reconciliation if it needs recursive covered-FUSE lookup; exact Root/epoch/Home lifetime and kernel alias checks. Add moving/deleted-parent, old fd/cwd, callback-deadlock and concurrent-native cases before activation. Kernel zero TTL returns0, so do not depend on a speculative same-jiffy grace period or a sleep-based fix.

- 2026-10-01 directory-worker mechanism: a test-only zero-TTL adapter around real OwnerFs pins one source directory/root and covered workspace fd. Off-receive-thread recursive lookup before GETATTR reply automatically repairs first old dirfd/cwd parent access, old-name replacement isolation, child data and a second native move. Disabled control fails ESTALE after name reuse; enabled control passes. Final helper exemptions are live TIDs only, verified against kernel headers; all actors/workers are reaped/joined before owned cleanup.
- Current full15-case portable pair: disabled13 PASS/2 FAIL, enabled14 PASS/1 FAIL on the same binary, with independent parent-mount/data cleanup checks. Production AfsFuse's old-directory case remains RED. This is a mechanism proof and cannot close Task3/4 or qualify performance.
- Next integration work: design/verify bounded source-object leases tied to lookup/forget and opened references, current Root/epoch/Home identity and pre-admission cache barrier; implement guarded production GETATTR worker dispatch and alias reconciliation only after deleted/nested/moved-parent, concurrent mutation and credentials cases. The prototype's single-object/fixed-authority helper logic must not become an undocumented production bypass. Node activation remains gated by pending cache/locks/durability/platform contracts.

- 2026-10-01 deleted-directory checkpoint: native move followed by deletion before first old-reference access is RED. Native dirfd retains RIGHT parent and fstat nlink0; path-based FUSE returns ENOENT. A test-only pinned-metadata control repairs fstat but leaves kernel parent LEFT. In-place deletion passes with pinned attributes, isolating object survival from parent relocation. Preserve both complete assertions and do not exclude deletion semantics.
- Final17-case same-binary four-control VM replay: default repair-off13/4, repair-on14/3; pinned-attribute repair-off14/3, repair-on15/2. All parent mountinfo and disposable-data checks pass independently. Production old-directory and deleted-after-move remain RED. No production activation or full-feature/performance claim.
- Next: find a safe kernel mechanism for a moved then unlinked directory's retained parent relationship; fd getattr alone and new-name lookup are insufficient. Keep directory object leases, current parent identity and bounded lifetime distinct. Do not resurrect a user-visible name as a repair side effect or copy an old cached parent over the native oracle.

- 2026-10-01 unlinked alias mechanism: exact helper lookup of a pinned deleted inode plus entry invalidation matches serial native parent/fstat/fd-path/getcwd/absent-name behavior. A kernel-stack-confirmed coalesced ordinary lookup returns ENOENT. Test-only fixed name, authority and one-object controls; no production integration.
- Atomicity rejection: a deterministic post-alias/pre-invalidation getcwd observer returns right/moving while a pinned native cwd observer returns ENOENT. Repeat confirms the same mismatch. Zero TTL and callback gating cannot control this kernel-only observer. Preserve this counterexample; do not promote the two-step synthetic alias mechanism into production.
- Final19-case same-binary controls: disabled13/6; repair with pinned attrs only14/5; unlinked helper+invalidation17/2, repeated. Remaining enabled failures are production old-directory handling and getcwd window. All mount/data cleanup checks pass independently; Python cwd-import and wait-symbol harness failures are retained separately.
- Next: identify and prove an atomic current-parent/deleted-alias mechanism, retaining both directory identity and absent-name semantics for kernel-only observers. Existing6.8 UAPI has no atomic reparent notification. Consider kernel/bridge alternatives as new feasibility work, not an assumed permission to change platform/architecture or weaken old-reference requirements. Other independent goal work may continue while design decisions remain pending.

- Final control correction verifies the actual deleted name LEFT for in-place deletion, RIGHT after native move. Matching19-case VM pair retains13/6 and14/5 without unlinked repair,13/6 and17/2 with it. The getcwd atomicity counterexample still fails; serial and coalesced controls pass. Preserve final source/SHA separately from preceding repeated evidence.

## Accepted native profile revision (2026-10-01)

The user explicitly accepted the previously presented close-to-open and retained-FUSE-directory constraints and supplied the main workflow: management creates/prepares workspace, then starts Agent and exposes the ready mount. Product contract and concrete cases now live in `docs/architecture/ownerfs-native-access.md`, linked from RFC, architecture, operations, acceptance and status. This changes the applicable native contract, not the overall delivery goal or other backends' defaults.

- Required: readiness must verify Root/epoch/Home/export identity and policy in the final Agent namespace before Agent cwd/dirfd acquisition; no inherited pre-bind FUSE directory reference. Creation ACK is not readiness. Explicit FUSE-only fallback must not claim native ready.
- Required: cross-path successful writer close followed by a fresh current-path open returns completed data/length; stale caches after reopen remain bugs. File fd identity survives rename/unlink/replacement. Ordinary native POSIX, cross-path locking, permission, supported mmap/sync and managed lifecycle remain gates.
- Accepted boundary: old cross-path readers need not update immediately; old FUSE cwd/dirfd getcwd/parent traversal/path display need not instantly follow native directory changes. Fresh file open relative to stale `..` is not current-tree revalidation; reacquire directory from stable workspace root. Native references retain native semantics.
- Historical first-parent, first-getcwd and deleted-alias/getcwd-window strong-equivalence diagnostics retain original assertions/logs. They explain the boundary and must not be silently counted as product acceptance passes; the current prototype still is not production integration. Stop seeking kernel extensions solely to enforce these now-excluded old-directory guarantees. Remaining lock/mmap mechanisms need their own evidence.
- Replace strong cross-path-live-cache gate with CTO plus object-lifetime tests. Add actual management ready-before-Agent case, immediate post-ready native file operations, final-namespace identity/failure-before-start, cached-reader close/reopen CTO, stable-root fresh directory lookup, replacement/unlink old file handles, sync recovery and managed busy/fencing cases. These are not yet implemented/qualified.
- Preserve all prior evidence with the contract/version it tested. Native full-feature and performance/PR delivery remain incomplete. Do not modify handoff without explicit request.

## Native cache integration slice

- Construction-only native-eligible OwnerFs policy now uses direct I/O and zero
  entry/attribute TTL; ordinary instances retain existing behavior. No config
  opt-in, Node activation or ready ACK is introduced by this slice.
- Actual VM journaled native/independent Home-FUSE fixture verifies CTO, retained
  file identity, stable-root fresh lookup, fallback and ready-before-Agent
  spawning; final selected case passes, ordinary-private control remains RED.
  Full production Node/P2P/lifecycle/lock/mmap and performance gates remain open.
- Native entry TTL behavior RED preceded the fix.355 library PASS/2 existing
  ignored, strict Clippy/fmt and VM source/binary/cleanup evidence are retained.

## Current-policy readiness slice

- First and duplicate activation now require live effective-policy verification
  before NativeActive ACK. Policy failures retain owned physical claims in
  Recovering; restoration/reconciliation then permits normal teardown.
- Two controller REDs and an actual changed-policy VM RED precede the fix.
  45 native targeted tests plus15 independently isolated VM foundation cases
  pass; recorded historical diagnostic exclusions retain their original tests.
- Production Node worker/ready interface and authority lease, Agent supervisor,
  network P2P locks/cache/mmap and full lifecycle/performance remain incomplete.

## Home kernel-flock slice

- Native-eligible OwnerFs now routes flock to duplicated actual Home backing
  descriptions. It never reopens by name; ordinary OwnerFs and DFS retain the
  existing userspace lock behavior. POSIX byte-range locks remain unresolved.
- A bounded OwnerFs-only coordinator serializes grant/cancel/release, checks
  closed file slots at each kernel attempt, and uses NB retries for native
  unlocks. Root invalidation and authenticated peer/session/final-file cleanup
  explicitly unlock pins; unacknowledged released grants become Cancelled.
- Initial two native-arbitration behavior REDs and a separate released-outcome
  RED precede their fixes.10 source regressions cover native/Home arbitration,
  waiting/cancel/close/revocation, peer authentication/session cleanup, shared
  upgrade and unlinked/replaced file identity. Actual VM mode0 control fails;
  mode1 passes the new kernel-flock case.16 selected VM foundation cases pass.
- Remaining: same-host old-FUSE/native POSIX process-owner boundary awaits the
  explicit user decision; do not silently replace it with an OFD lock. Node,
  final Agent namespace/authority readiness, network P2P, mmap, full managed
  lifecycle and paired native-ext4 performance gates remain open.

## Home authority admission slice

- Issue an opaque, instance-bound export permit only from a current local Home
  grant with Lookup/Read/Write and construction-time native cache eligibility.
  Pin the exact confined backing directory; recheck current namespace, grant
  fencing/session/rights, configured directory and source object identity.
- Guard physical manager activation with the registered Root/epoch/Home/source
  match and authority checks before and after attachment. Lost authority allows
  only normal owned-export teardown; EBUSY retains its claim and Draining state.
  A foreign OwnerFs caller must be rejected before any cleanup mutation.
- This is a Home admission bridge, not a Node worker, final Agent namespace
  readiness, continuous authority lease or reclamation API. Connecting the
  pinned source to production descriptor preparation and actual VM admission
  tests is still required. Full Agent/Meta lifecycle, POSIX/P2P/mmap and paired
  native-ext4 performance gates remain open.

## Home authority teardown checkpoint

- [x] Connect the opaque Home permit's exact directory fd to Linux preparation;
  retain a metadata-only Root anchor until normal teardown/preparation release.
- [x] Reproduce implicit kernel submount loss on revoked FUSE Root revalidation,
  then retain positive mountpoint identity without retaining data authority.
- [x] Reproduce old FUSE file reads after revocation; native-eligible open handles
  now capture Home authority and re-admit data/metadata mutations. Preserve
  previously accepted dirty-data sync and retirement cleanup. Ordinary defaults
  remain unchanged.
- [x] Replay the real VM revoke/busy/normal-detach case and all17 required
  foundation cases. Historical stronger-directory diagnostics remain NOT_RUN.
- [ ] Wire production Node/Agent readiness and supervised process fencing,
  recovered current permits, full Meta deletion/reclaim/switch lifecycle,
  applicable POSIX/P2P/mmap and paired native-ext4 performance. Mountpoint
  metadata retention and a revoked RootGrant do not stop native fd I/O.

## Native lock Home authority and release outcome slice

- [x] Reproduce four behavior failures using production Home recovery and an
  actual failed kernel unlock with retained original lock and independent native
  contender. Preserve REDs and cleanup evidence.
- [x] Admit the captured native file authority before choosing a target or
  cloning its descriptor, bind table identity to the full current Home grant,
  reject retired targets after wait and allow fresh recovered authority.
- [x] Preserve granted outcomes/pins when normal owner unlock fails; only a
  successful kernel release changes granted to cancelled.
- [x] Run32 targeted native tests and all-feature library385 PASS/2 existing
  ignored, with fmt check PASS. Preserve exact source snapshot and logs.
- [x] Complete candidate strict lint, root-project checks and independent VM
  replay:17 foundation cases and all4 new regressions on ext4 PASS. Independently
  inspect binary/archive identity and unchanged mounts/empty temporary data.
  These Home/kernel fixtures do not qualify actual P2P or Agent lifecycle.

## Native flock retirement and errno slice

- [x] Reproduce retained outcome/retired/session state making invalidated native
  tables unreclaimable, including all512 inode-table slots and blocked waiters.
- [x] Preserve permanent replay fencing while clearing obsolete terminal/session
  history. Failed kernel unlocks retain their descriptor pins and prevent idle
  reclamation; later session cleanup can retry without repopulating history.
- [x] Preserve kernel errno through acquisition/unlock and public OwnerFs error
  conversion. Shared LockError gains a kernel-errno variant; the shared userspace
  model and DFS do not emit it or change their state transitions. Existing Node
  VFS unsupported classification carries ENOSYS without changing the catalog.
- [x] Run all-feature library regressions381 PASS/2 existing ignored and current
  VM foundation17 PASS. Full-project and lint verification outcomes belong in
  the corresponding evidence checkpoint; none prove Node/Agent readiness.
- [ ] Before Node native admission, consume current authority control and compare
  exact Root/epoch/Home session/generation on revocation. The existing Meta
  WatchRootCommands RPC returns one finite filtered batch, not a live watch.
  Cursor/high-watermark progress and compaction must be handled explicitly;
  EOF or an empty response cannot by itself establish a healthy ongoing lease.
- [ ] Complete native lock authority revalidation, precise failed-release terminal
  outcomes, applicable POSIX ownership and actual network peer/fault coverage.
  Production Agent fencing, complete lifecycle and paired ext4 performance
  remain required by the original goal.
