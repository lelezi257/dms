# Implementation Status

Last updated: 2026-10-01.

This page is the only implementation-status page. Architecture pages describe the accepted target design.

The [delivery acceptance contract](acceptance.md) requires JuiceFS default same-mount visibility and close-to-open semantics. A dedicated Linux environment, suite manifest, runner and acceptance Skill are prepared for development; reference suites and comparator prerequisites are still incomplete. Short regressions are not proof that release gates pass.

## Current Capability Matrix

| Area | Status | Evidence and limit |
| --- | --- | --- |
| Process foundation | Implemented foundation | `afs-meta`, `afs-node`, config, logging, metrics, REST, gRPC, FUSE wiring and shared errors exist. |
| FUSE module | Implemented foundation | Shared FUSE module supports separate OwnerFs and DFS sessions. |
| MetaStore | Experimental | memory, local-file, etcd and Redis share one store interface. Redis uses atomic full-snapshot CAS and checks AOF always/noeviction/no TTL; real Redis CAS, configuration rejection and AOF restart short regressions pass; actual Meta-process recovery/fault parity remains unverified. `MetaReadView` pins one acknowledged revision without requiring native multi-record backend transactions. Meta HA is deferred. |
| OwnerFs | Experimental | Home-local files, P2P access to Home, root grants, handle checks and cleanup exist. Dirty local close flush performs data sync; remote flush reaches the Home boundary. Identified Linux TLS/etcd-backed local and remote-Home mounts pass 11 POSIX and 4 multi-user short checks. Ordinary test fixtures live inside a workspace; deleting the workspace authority root is not implemented as ordinary rmdir. Full fault/POSIX coverage remains incomplete. |
| OwnerFs native bind | Design accepted; integration incomplete | Verified mount/controller/journal primitives, construction-only native cache policy, a passing Home-FUSE/native close-to-open fixture, kernel-backed Home flock with bounded invalidation cleanup and preserved kernel errors, and source-bound Home-grant preparation exist on the isolated feature branch. All17 selected VM foundation cases pass, including grant revocation with busy-mount retention, denied old FUSE reads and normal teardown; this is not actual Agent process fencing. The native flock fixture verifies actual VM FUSE/native mutual exclusion and retained file identity; authenticated Home peer/session cleanup has source regressions, not network-P2P qualification. Node native admission and Agent readiness/start ordering, native POSIX-lock/mmap and full P2P/lifecycle integration, and native-ext4 performance qualification remain incomplete. The [native contract and practical cases](architecture/ownerfs-native-access.md) specify managed prepare-before-Agent startup, cross-path close-to-open and retained FUSE directory-reference limits. Historical stronger-contract diagnostic failures are not product passes or blockers for the newly accepted directory boundary. |
| DFS R=1 write path | Experimental | DFS mount, shared inode dirty state, serialized mutation/commit, exact uncertain-request replay, truncate and sparse-file paths exist for local owner scenarios. Definite commit rejection blocks later mutations until owner recovery. Identified A/B memory-backed mounts pass short remote-owner writes, handleless resize, same-mount dirty reads and owner-handover scenarios. Complete fault and backend coverage remains unqualified. |
| FileVersion and layout model | Experimental | `FileVersion`, `LayoutRoot`, `Extent`, `ChunkObject`, CAS commit and base-chunk inheritance exist. Extent tree and compaction policy are not complete. |
| Replication | Experimental gRPC chain | Configurable replica targets/synchronous minimum, global placement, authenticated receiver write grants, bounded staging and ordered durable chain acknowledgements exist. Real localhost missing-tail/exact-retry tests pass; full multi-VM faults and repair workers remain incomplete; RDMA payload adapters have short proof only. |
| Local chunk engine | Experimental foundation | BLAKE3 identity, per-chunk files, local catalog, finalize and startup recovery exist. Pack backend, relocation, GC and full crash matrix are not complete. |
| File reads and default consistency | Local experimental | Ordinary readonly handles share the local inode dirty view; each read pairs one base version/layout. Writable close flush commits recovery state, retains uncertain requests and reports errors. DFS cached write-through passes a real Linux warm readonly/MAP_SHARED-read/overwrite/append/shrink-grow slice. Remote owner forwarding of data and attributes passes the identified eight-case A/B short probe, including write-only providers and retained-state handover. Full concurrency and permission matrices remain incomplete. `DfsReadEngine`, bounded peer batches and connection pool exist; authenticated batched Meta read-grant validation and bounded receiver caches pass Linux source and localhost streaming regressions; the identified A/B handover and peer-read short probe passes. Full security and fault matrices remain incomplete. |
| Native SDK | Foundation | `DfsLocalData` protocol and typed DFS client identity framework exist. The default Node service returns `UNIMPLEMENTED`; diagnostic `LocalData` remains separate and does not become OwnerFs. |
| RDMA | Experimental product adapters | An identified Linux RXE product-adapter probe verifies two4MiB durable replica transfers and75000B peer reads with zero gRPC file payload, exact content/retry and denied forged authority. Actual cross-VM RXE FUSE R2 synchronous writes and R1 peer reads each verify4194321 bytes with zero gRPC file payload; the B replica survives restart. Complete fallback, fault and security matrices remain unverified. |
| External spill | Not implemented | Design exists for `ExternalCommitted`; no product spill path is complete. |

## Current Validation Checkpoint

The native-bind goal now follows [architecture validation → measured ext4/native
bind/MooseFS → production integration](../development/native-bind-validation-plan.md).
Architecture acceptance has not passed. Same-path and retained-reference mount
primitives have evidence, but the same-process POSIX lock-owner conflict has no
approved exemption, current-candidate network-P2P/mmap and managed switch/fencing
need decisive combination evidence, and performance has not been measured.
Third-stage handle-cleanup WIP is preserved separately; verified source remains
caec0fa. Historical tests below keep their stated scope and do not establish
stage completion. The stock MooseFS durable-write baseline remains unqualified.

An additional A1 combination probe on the independent ext4 VM verifies actual
kernel EPERM on physical activation, FuseOnly/no native claim, continued FUSE
access to the same prepared backing, successful retry and normal-detach fallback.
Retained FUSE fd/dirfd operations stay on that backing after native attachment;
a ready-after-activation child observes the verified native inode/namespace.
This uses current Home authority with an in-process Meta fixture. It does not
qualify paused in-flight callbacks or production Node/P2P READY publication.

The current A1 request-path probe on the independent ext4 VM confirms native
file operations but exposes remaining ancestor cost:16 rounds using absolute
workspace paths issue96 root GETATTR and96 workspace-root LOOKUP requests.
Ready native cwd/dirfd-relative operations issue zero, with a retained FUSE dirfd
positive control. No timings were collected and no native-performance conclusion
is established. [Raw identities and observer failure/replay](../development/native-bind-evidence.md)
remain recorded; performance must include both path forms. This probe adds only
experiment code and leaves production behavior and parked handle cleanup intact.

The native-bind branch now retains local/peer xattr admission through syscall
completion and captures opening authority on native Home directory handles.
Readdir validates that capture, including empty directories, and remains counted
through enumeration/peer response construction. All4 original behavior failures
become green;414 library tests (two existing ignored), strict Clippy/fmt and
independent ext4 VM replay pass. The previous opendir publication assessment is
corrected: an existing second RootUse already covered physical open/publication.
Whole-operation/path-object audit, actual handle/dirty cleanup, Node/Agent
fencing, normal detach/ACK and full semantics/performance remain open; no native
READY or PR completion is claimed.

The native-bind branch now counts native Home local/peer lock calls through
their actual completion and offers exact-authority lock cleanup for a current
refusal. Two original blocked-wait counter failures become green; all6 focused
cases,410 library tests (two existing ignored), strict Clippy/fmt, root-project
tests, three feature builds and independent ext4 VM replay pass. Fresh actual
VM foundation17 also passes. Actual failed unlock retains its pin/errno until retry;
another workspace or recovered authority is not unlocked by an old command.
This is not open-handle, native Agent, mount or ACK drainage. Node command
consumption, complete semantics/performance and the no-merge PR remain open.

The native-bind branch now has a command-scoped RootManager refusal and actual
admitted-operation barrier. Five focused regressions,404 all-feature library
tests (two existing ignored), strict workspace Clippy/fmt, root-project tests,
three feature builds and independent ext4 VM replay pass. Fresh actual VM
foundation17 also passes. The project run includes the unchanged DFS expiry
case, but does not establish that earlier WSL clock anomalies are fixed; prior
failures and ignored/unrun scope remain in the evidence record. Delayed or
foreign command identities cannot close the current
grant; retired cache objects cannot prove the replacement is drained. This
interface is not yet called by the production Node worker and does not prove
open-handle/lock/native-Agent drainage, normal detach, durable cursor state or
ACK completion. Full semantic/performance gates and the no-merge PR remain open.

The latest native-bind control prerequisite adds a current-session command poll
with pinned prefix cursors and exact command/ACK/replay checks.399 all-feature
library tests PASS (two existing ignored), strict Clippy/fmt and three feature
compilation variants PASS, and45 native contracts PASS. Independent VM replay
passes14 control cases plus4 lock regressions; the separate39 Meta contracts also
PASS on VM. The WSL whole-project command remains FAIL at the unchanged DFS
expiry case; its unexecuted remainder/doc tests are not thereby qualified.
Failed attempts and original/corrected timeout semantics retain their evidence.
The poll is one snapshot, not an ongoing lease; Node command consumption,
managed Agent fencing/readiness, normal teardown and truthful ACK production
remain unwired. Full semantic/performance gates and the no-merge PR remain open.

The previous native lock authority/release candidate has385 passing all-feature
library tests (two existing ignored), including32 targeted native cases and
four regressions that reproduced the original authority/outcome failures.
Fmt, strict workspace Clippy and root-project all-feature tests pass. Fresh
independent VM replay passes17 foundation cases and all4 new regressions on
ext4; binaries, raw outcomes and independently checked mount/data cleanup retain
their exact identities in the evidence index. Node/Agent lifecycle, applicable POSIX/network-P2P
and native-ext4 performance qualification remain open; no PR has been created.
The checkpoints below are historical evidence with their original scope.

The isolated native-bind branch has10 passing Home flock regressions and16
passing VM mount/cache/readiness/local-flock foundation cases. Its strict
workspace/all-target/all-feature Clippy and fmt checks pass. The broader
workspace/all-feature test command is not green: unchanged vendored fuser
`ll::request::tests::init` and `reply::test::reply_create` fail under its higher
ABI feature set, also reproduced by running that package alone. See
[native evidence](../development/native-bind-evidence.md) for raw identities and
scope; this checkpoint does not establish full native or release acceptance.

The subsequent Home-authority bridge has8 passing source regressions,373
passing library tests (two existing ignored),45 passing native controller,
journal and event contracts, and passing strict workspace Clippy/fmt. The new
project-wide test attempt FAILS at
`dfs_renewal_cannot_resurrect_expired_or_reassigned_authority`; it also fails in
isolation. Read-only WSL clock measurements observed realtime jumps while
monotonic time advanced normally. This is retained diagnostic evidence, not a
waived assertion or a green project run. The remaining project suites/doc tests
were not reached in that run. The8 new regressions use a mount-backend probe;
they do not extend the prior16 actual VM passes to Home/Agent admission.

The current controller parses literal/basic single-line listen and mount values, preserves quoted content and rejects malformed listen values before launch. [Startup and A/B evidence](../development/evidence/20261001-toml-startup/README.md) retains the original false-readiness failure, a failing Linux regression and18 passing regression groups, including54 existing native lifecycle command records. Single/double quotes with comments and malformed/out-of-range preflight checks pass. Shellcheck was unavailable; Unicode escapes and multiline deployment scalars are not supported by this reader.

New A/B runtimes use the v51 Rust binaries below and controller945cba0c. Memory/R1/gRPC/TLS OwnerFs/DFS pass10 consistency scenarios and7 cross-node lock steps per backend with the unchanged35/55-second bounds. Root matches248 captured product PID/executable/hash/start-tick records,143 Rust compile inputs, current config/controller/worker hashes, exact FUSE mounts and live readiness. Earlier v48 processes retain their original identities. This is a short development matrix; the full upstream run below remains qualified only for v48, and all69 formal release cases remain NOT_RUN with environment PREPARING. Host preparation and verifier-selection errors remain explicit unqualified attempts.

Candidate v51 passes the coherent Linux gate:352 library tests (two explicit environmental ignores),57 interface contracts,four shared errors,nine local API tests,five privileged actual FUSE tests,formatting,strict Clippy,feature checks and binary build. All143 compile inputs match. [Signal-deadline evidence](../development/evidence/20261001-shutdown-signal/README.md) preserves the failing blocked-executor regression, targeted signal tests and read-only review limits. The production signal observer registers before the business executor and arms the native deadline independently of its scheduling.

An isolated A runtime uses identified v51 binaries, memory Meta,R1,gRPC/TLS and separate OwnerFs/DFS mounts. Normal dirty shutdown returns0 and restart first-read matches. With the same Meta paused before a four-byte accepted dirty write, direct Node SIGTERM reaches a matching exit receipt in5.034s: explicit drain failure1, not forced124. The controller returns1 and status/receipt agree; after the same Meta resumes, a fresh Node first-read returns the preceding fsync watermark. Final stops return0 and leave no mounts/listeners. Root independently verifies guest report, executable/config hashes,four receipts and recovered bytes. A host collection path error is recorded as INCONCLUSIVE; the actual report is recollected and matched to the direct guest copy. The native blocked-executor regression separately verifies forced124. Earlier20.124s is not explained by this run, and the differing procedures do not establish a performance improvement. Full DEP/REL and cooperative shutdown qualification remain open. Preserved v48 integration below retains its own identity; the new A/B matrix above uses v51.

Candidate v49 passes the coherent Linux gate:350 library tests (two explicit environmental ignores),57 interface contracts,four shared errors,nine local API tests,five privileged actual FUSE tests,formatting,strict Clippy,feature checks and binary build. All143 compile inputs match. [Writeback evidence](../development/evidence/20261001-writeback/README.md) retains three failing old-behavior injections and the initial narrow timeout-fixture assertion.

Background DFS writeback examines at most64 inodes per pass, advances a fair cursor by actual progress, and shares one250ms admission/Meta RPC budget. Busy inodes are skipped. Prepared-unsent exact requests remain pending; issued timeout remains unknown; foreground sync keeps its ordinary budget. Table snapshot/sort and started physical IO are not strictly wall-clock bounded. Placement/R=N and whole-maintenance budget propagation remain open. v49 has only the isolated A controller lifecycle proof below; preserved A/B integration remains v48.

Identified A/B v48 memory-backed R1/gRPC mounts pass ten consistency scenarios and seven distributed-lock steps per backend with unchanged35/55-second bounds. All248 captured process records match. The full B DFS pjdfstest run, guarded by strict A Meta/B Node identity, passes236 files/8819 TAP checks with zero unexpected failures/skips and28 upstream TODO in1253.484s under the original1800s bound. Seven retrieved worker/host artifact entries match bytes/SHA. This qualifies the specific v48 development matrix, not v49 or unrun backends/transports/release cases. The first preflight-only attempt remains BLOCKED.

The earlier isolated v48 held-fd healthy shutdown/restart and paused-Meta failure proof remain in [shutdown evidence](../development/evidence/20261001-node-shutdown/README.md). The15-second native guard reports forced exit124; it does not prove every worker stops cooperatively. The original native controller false-clean fixture is retained as FAIL. [Controller evidence](../development/evidence/20261001-processctl/README.md) qualifies exact wait-status propagation:16 Linux regression groups/54 recorded CLI commands, native0/1/124 and SIGKILL137, unknown receipts, readiness, startup recovery and alias concurrency. An isolated actual A runtime with v49 binaries passes normal dirty shutdown/restart and a post-pause dirty-write forced-exit proof. Controller status and four receipt sets match actual exits; exact Meta identity resumes and first read returns the fsync watermark. Stop in the paused-Meta case takes20.124s and returns124; this does not establish a15-second signal-to-exit bound or cooperative completion. Formal DEP qualification remains open.

Earlier v45 passes337 library tests with the same contract/error/local-API/FUSE counts. [Owner restart evidence](../development/evidence/20261001-owner-recovery/README.md) preserves two failing original-behavior regressions, the original v44 runtime failure and the repaired source/runtime proof.

Actual A/B v45 memory-backed mounts pass a64MiB R1 owner Node SIGKILL slice: Meta stays live; the same Node binary/config/disk restarts without manual socket removal; the first fresh B read matches length and SHA256. B close during owner unavailability returns explicit EIO. One observed queued old-session cleanup retires after replacement and before its original expiry. Source/receiver grant validation derives the new serving epoch without modifying the durable receipt. This does not prove Meta restart durability, disk loss or the complete lifecycle fault matrix.

After restart, v45 A/B passes all ten consistency scenarios and seven cross-node lock steps per backend, retaining the original35-second wait and55-second interrupt bounds. All248 captured product identity records match. Earlier [v43 open admission/provider lifetime](../development/evidence/20261001-owner-open/README.md) and [v44 session retirement](../development/evidence/20261001-owner-retirement/README.md) results retain their own identities. The provider idle wait is bounded and timeout retains exact guarded cleanup debt; the total dirty-writeback/shutdown budget and current full-suite/backend matrices remain unqualified.

[Earlier v41 release-retry evidence](../development/evidence/20261001-release-retry/README.md) verifies one attempt per identity per maintenance pass, the64-entry cap and fair rotation. The [v40 full remote run](../development/evidence/20261001-remote-full-timeout/README.md) reaches the original1800-second bound:170 observed completed files,66 unobserved,7961 checks and zero unexpected assertions. It remains BLOCKED. Original accounting is preserved and corrected separately;51 Linux driver/identity selftests pass. Earlier v40/v37 results below retain their own binary identities.

Identified A/B v37 memory-backed OwnerFs/DFS mounts pass ten cross-mount consistency scenarios. These include same-mount visibility, close-to-open, remote owner writes, local and remote-Home overwrite rename with a surviving hardlink, owner handover, retained local state and an existing write-only remote handle surviving handleless resize. Assertions do not poll until stale data disappears. [Earlier v36 evidence](../development/evidence/20261001-authority/README.md) separately records seven-step OwnerFs/DFS cross-mount lock checks with 35-second waits, a two-step exact-target 18-second Meta pause and 500 DFS/ext4 random operations under a 180-second functional bound. These earlier runs retain their own binary identities. The v37 OwnerFs seven-step lock rerun passes. The concurrent v37 DFS rerun has six PASS and one FAIL: the remote interrupted blocking wait does not complete within 55 seconds. The original failure is preserved. Identified v40 A/B mounts pass ten consistency scenarios and seven lock steps per backend. The DFS lock rerun overlaps a synthetic namespace load for its entire duration; the load completes 3842 operations with stable identity. The same 35-second wait and 55-second interruption bound are retained. This focused repair does not replace a complete upstream-suite-plus-lock rerun or sustained lifecycle fault qualification.

The complete v37 OwnerFs pjdfstest run executes all 236 files and accounts for 8819 TAP checks, with zero unexpected failures/skips and 28 upstream TODO. Before/after process, configuration and mount identity checks pass. The preserved v36 full run has ten unexpected `rename/23.t` failures: the surviving hardlink returned `ESTALE`. Two targeted regressions fail with the original rename logic and pass after canonical path rebind. The complete v37 DFS run also passes all 236 files / 8819 checks with zero unexpected failures/skips, 28 original TODO, and all pre/post identity checks. [The earlier DFS v26 full result](../development/evidence/20260930-resume/README.md) binds its own binary and does not qualify the current candidate or unrun backend variants. Full-suite success here covers that v37 development matrix, not v45 or the entire POSIX release contract. The current candidate full-suite matrix remains unrun.

The remote STD-01 driver passes 48 Linux selftests and an actual B DFS smoke run with strict A Meta/B Node identity: four files / 241 checks, zero unexpected failures/skips. Pre/post process, configuration, TLS, endpoint, mount and guest-ext4 evidence checks pass. All seven retrieved raw artifact manifest entries match exact bytes. The smoke profile accounts for 232 discovered files that it did not run. [Driver evidence](../development/evidence/20261001-remote-standard/README.md) does not qualify the full remote or backend matrix.

The formal 69-case release manifest remains NOT_RUN and the environment lock PREPARING. No mandatory release or performance gate is declared complete by these short checks.

## Known Open Items

- Complete the wider owner-open ACK-loss, remote cleanup/resource and network-fault matrix, cooperative whole-writeback-pass budgets and complete shutdown/controller deployment matrices. The native process deadline reports forced failure; it does not prove every worker cooperatively stops.
- Complete the distributed-lock resource and long-run lifecycle matrix. Short actual A/B contention, cancellation, delayed waits and close/session checks have evidence; capacity and exact retired identity still require sustained product validation.
- Run full applicable POSIX matrices for the current candidate and required backend variants without adding exclusions; complete remote-owner consistency, permissions, namespace, mmap, xattr, ACL and directory durability coverage.
- Complete cross-node DFS file-operation and lock authority, stale owner rejection and uncertain-result fault verification; preserve exact pending identity and inode serialization.
- Finish R=N multi-VM faults, repair workers, placement and source-loss behavior, full RDMA/fallback/security/lifetime tests and resource accounting.
- Core development uses memory Meta. Complete etcd/Redis persistence, parity and recovery separately after core functionality/performance development. Existing etcd integration uses an approved enlarged request limit; full-snapshot retention and serialization cost remain open.
- Complete local chunk crash/reconcile/GC, pack/relocation, compaction and large-file bounded-memory/capacity/corruption cases.
- Validate actual Home backend availability, full OPS diagnostics/readiness/backpressure and DEP clean/offline/idempotent installation, shutdown and restart matrices. Failed remote release can leave an owner-side handle requiring retry/session cleanup; local provider removal/restoration has regressions, while full remote cleanup fault coverage remains open.
- Qualify fair MooseFS/3FS baselines before performance tuning and paired performance claims. Stock MooseFS strong-durability matching and the ARM64 patched 3FS reference remain open prerequisites.
- Run final 8 GiB, full FSx/random-operation seeds and eight-hour soak; source or short-mount regression success is insufficient.
- Read-grant protocol changes require coordinated upgrade: request-level grant field 5 is reserved, each operation uses grant field 7, and `DfsReadGrant.caller_epoch` remains field 5. Existing persisted copy records require explicit migration to `CopyLocation`; no automatic old-format migration is provided.
- DFS SDK, product cache/spill and Meta election/HA remain outside the first-stage gates, as recorded in the [post-acceptance TODO](acceptance.md#10-第一阶段验收后-todo).
