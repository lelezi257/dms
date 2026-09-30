# OwnerFs native bind implementation plan

> For agentic workers: use superpowers:executing-plans inline. Preserve the existing RFC and completed probes. Use failing Linux regressions before implementation. Submit a PR for review; do not merge it.

**Goal:** Complete the same-path native OwnerFs feature with correct applicable filesystem/lifecycle semantics and measured native-ext4-level local performance. A mount-only manager is an intermediate result, not completion.

**Architecture:** OwnerFs remains the authority and file/P2P backend. A dedicated native manager controls verified workspace exports in the managed namespace. FUSE is a consistent fallback and transition path to the same ext4 object. Existing OwnerFs/DFS state and RPC boundaries remain separate.

**Tech stack:** Rust1.95, existing libc/serde/fuser, Linux guest ext4, mount identities and descriptor-based mount API; independent Home/peer Node mounts. No new third-party dependency unless a demonstrated gap requires it.

**Spec:** `docs/rfcs/0001-ownerfs-native-bind-mount.md`, user-approved complete-semantics/native-ext4 goals, GitHub issue #42. Baseline main `6bcabe8f30040bc6cc3b518bd271e7e2461e1e1d`.

## Global constraints

- Retain completed probe evidence. Rerun only affected assumptions and product regressions; do not count historical failures as fixes.
- Preserve `/ownerfs/<workspace>`, one backing directory, fixed Home and managed Agent lifecycle. No copy/symlink substitution.
- Never wait for same-path bind before the FUSE mkdir reply. Old fd/dirfd/cwd do not become native merely because a mount completed.
- No silent cache, lock, permissions, mmap or durability regression; complete applicable acceptance cases remain requirements.
- Match root/epoch/Home session/namespace and physical directory/mount identities. Unrelated mounts/processes/data are never cleanup targets.
- Default configuration preserves existing behavior; enable native explicitly. No hot conversion of existing cached/mapped references.
- Failure/busy/unknown states stay visible. Lazy detach is not access revocation or permission to delete/reuse data.
- VM ext4 holds tested data. WSL is build/source regression only. x86 feature evidence does not qualify original ARM64 formal release or MooseFS/3FS baselines.
- Pair native and ext4 workloads on the same VM/volume with identical data, flags, sync policy and host-load accounting; report every repetition and uncertainty, not a selected fastest result or artificial tolerance.
- Current acceptance close-sync contract conflicts with the older RFC's native plain-close statement. An explicit user decision is pending; do not activate a native path that silently violates the final selected contract.

## Review focus

1. An old asynchronous operation arrives after cancellation or same-name/epoch change: reject it without touching the newer root.
2. Target has stacked/foreign mount, symlink replacement or wrong namespace: refuse mount/unmount; never claim native readiness from path text alone.
3. Native write after old FUSE fd open, rename/unlink/replacement, or held mmap: content and object identity must not become stale or rebound by path.
4. Native fcntl/flock competes with local/remote FUSE; close, cancellation and dead peers: use the Home kernel inode and retain precise owner/waiter lifetime.
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
- [ ] Configure the independent manager/worker and publish a trusted ready-workspace event after mkdir reply. Do not hold authority or FUSE locks while executing mount.
- [ ] Repair both RFC stale-cache counterexamples. Validate held file/dir handles and mappings; no partial enablement while an unresolved cache/semantic case remains.
- [ ] Verify real local-native/old-local-FUSE/remote-FUSE/P2P against the same Home objects, plus simultaneous DFS mount isolation.

## Task 4: kernel-visible locks and complete supported file semantics

Files: new OwnerFs-native lock mechanism where possible; narrow integration in OwnerFs and peer contracts, avoiding shared DFS lock behavior changes. Extend existing metadata interfaces only for proven missing operations.

- [ ] Add RED actual-native-versus-FUSE/peer fcntl/flock tests with native/native controls; cover range replacement, read/write locks, fork/dup/close, cancel before/after grant, long waits and peer cleanup.
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
