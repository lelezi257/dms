# Historical implementation decisions for final review

Copied from the research execution record at this checkpoint. Earlier test counts and pending statements are historical. Current qualification is in README.md and docs/status.md.

# Final-review change record

Record significant architecture/module/interface/data-format changes here. Routine bug fixes use Git history and case evidence.

| ID | Problem / decision | Affected interfaces | Compatibility / alternatives | Evidence |
| --- | --- | --- | --- | --- |
| D001 | Layer implementation/validation guidance outside AGENTS to keep instructions small | AGENTS, development guides; no runtime API change | Acceptance remains the sole release contract | rules and execution entry |
| D003 | Ordinary readonly handles identify an inode rather than pinning a lifetime FileVersion; reads snapshot matching base version/layout and current overlays. Writable flush commits a Close/DataOnly barrier; release remains cleanup | DFS private handle/state helpers; shared Backend flush contract; existing FUSE interface unchanged | Removes incorrect OpenedFileVersion wrapper, keeps fixed ReadBatch. O_DSYNC full-bit check corrected. No blob/snapshot API introduced | p2a-dfs-before-online.log, p2a-dfs-after.log, p2b-vfs-before.log, p2b-vfs-after-final.log |
| D004 | DFS uses cached write-through without WRITEBACK_CACHE or KEEP_CACHE. Same-mount syscall writes update kernel pages; current backend view fills cache misses | Shared FUSE per-session cached_io policy, DFS mount only | OwnerFs policy preserved. Avoid synchronous invalidation inside WRITE callback; out-of-band/remote lifetime refresh is not inferred. Mapping-write/POSIX/remote paths still need their own tests | p2b-vfs-design.md, p2b-kernel-before.log, p2b-kernel-resident.log |
| D002 | Add Redis as a durable Meta Store backend using the existing opaque full-snapshot CAS boundary | `MetaStoreBackend::Redis`, `--redis-endpoint`, `src/meta/store/redis.rs`, Redis key `afs:meta:snapshot` hash fields `version`/`payload` | Redis does not store business records or replace Store semantics; unsafe Redis configs are rejected before Meta starts; no Meta HA or fencing added | `evidence/afs-delivery/p2e/design.md`; Linux targeted tests |

## D005 — Multi-user FUSE and extended attributes

- Reason: FUN-09/STD require multiple application identities and Linux xattrs. A mount restricted to the daemon uid and missing xattr callbacks cannot meet this contract.
- Design: keep the common FUSE/VFS boundary; expose get/list/set/remove xattrs with exact size-query/ERANGE and CREATE/REPLACE rules. Product mounts use AllowOther plus DefaultPermissions. Backend and Peer authorization still validate the originating caller; kernel checking is not a replacement for remote authorization.
- Root operation: installed Node processes run under the root installer process model. Non-root development mounts additionally require user_allow_other in guest fuse.conf; inability to mount must report an error, never silently omit permission checking.
- Evidence: evidence/afs-delivery/p2b-xattr-before.log records the original real Linux FUSE xattr rejection. New backend integration and multi-UID checks remain pending.

## D006 — Sparse allocation is distinct from EOF

- Reason: FUSE previously reported blocks=ceil(logical_length/512), misrepresenting holes as allocated data.
- Design: FileAttributes.blocks carries actual allocation in POSIX 512-byte units. OwnerFs derives it from local metadata and preserves it across Peer transport. DFS derives allocation from data extents and dirty data, without charging holes or replica copies as logical-file allocation. Common FUSE passes this value through.
- Validation: shared sparse-attribute regression added; current backend integration and merged Linux validation are pending.

## E001 — Rejected validation environment

- P2c Meta namespace agent ran formatting/protocol/error-crate compilation tests on macOS despite Linux-only rules. Original logs are retained in evidence/p2c-meta-attrs/local-validation.log; those runs are not accepted validation.
- Correction: all Rust validation reruns on afs-build; root requires a fresh merged Linux build and contracts before publishing any verified milestone. No release gate is marked passed by these runs.

## D007 — Metadata caller supplementary groups

- Problem: uid/gid-only RequestContext loses supplementary group membership; legitimate owner chgrp and group-authorized user xattrs would be denied or incorrectly use daemon identity.
- Decision: add supplementary_gids to the existing RequestContext (Clone, no Copy), preserve the existing Meta CallerContext wire boundary. FUSE metadata callbacks read Linux caller /proc thread credentials without a long-lived group cache; require matching kernel fsuid/fsgid, stable process start, and matching group observations. Missing or changed observations grant no additional groups. DefaultPermissions remains the kernel authorization gate. Ordinary read/write does not read /proc.
- Scope: shared FUSE/VFS caller data plus Owner/DFS peer/Meta serialization and metadata authorization; no new module or external dependency. Direct Backend callers must supply their authenticated caller groups.
- Compatibility: internal Rust struct initializer change; caller wire additions are additive. Supplementary group changes during an in-flight request can produce conservative permission denial; no claim of perfect credential-snapshot reconstruction.
- Verification: pending merged Linux compile/targeted tests and real multi-UID mount probes; not release PASS.

## D008 — Exact commit replay across inode metadata revisions

- Problem: an unrelated namespace/xattr mutation can advance inode revision while a data commit is in flight. Rewriting the expected revision during an uncertain retry would change the original request identity.
- Decision: retain the exact Node request. Meta uses the expected inode revision as a lower bound and CASes the latest read inode, while checking the exact expected file head and current write lease. New mode/xattrs/nlink remain in the latest record. Future revisions and stale heads remain rejected.
- Scope: Meta data/full-metadata commit validation and Node uncertain-request replay; ordinary namespace/attribute mutation CAS remains strict. No dependency or module split.
- Evidence: Linux Meta18 and the unknown-ACK replay regression in p2c-merged-linux-v3.log. Concurrent timestamp mutation and complete fault matrices still require testing.

## D009 — Regular mknod through the shared FUSE create boundary

- Decision: regular-file mknod reuses Backend create, flush and release because mknod has no application open fd. Unsupported special file kinds return explicit errors.
- Evidence: both real OwnerFs/DFS POSIX short probes passed regular mknod read/write/sync. This does not claim all special-file extensions are supported.

## D010 — Global placement and authenticated replica writes

- Reason: node-local placement cannot choose distinct live peers for configurable replica counts, and a receiver cannot trust a caller-supplied replication chain.
- Decision: Meta validates the frozen ordered targets against current node/device identities and the replication-group epoch. A receiver gets an authenticated write grant before persisting immutable bytes. Catalog revisions are freshness floors, not immutable transfer identities; a heartbeat must not alter an exact retry request.
- Scope: existing Meta/Peer RPCs, Node epoch registration, shared chunk staging budget and replica data plane. R=1 uses local persistence without Peer RPC; R=N persists the required synchronous chain prefix and returns ordered acknowledgements. Wire additions are additive; no file-layout change or new storage engine.
- RPC cost: remote chain links carry bounded streamed bytes, and each receiver currently validates authority through Meta before persistence. This extra control RPC is explicit and requires later profiling; it is not claimed to be optimal.
- Evidence: `evidence/afs-delivery/p3a-chain/README.md`; strict Linux Clippy, 156 library tests, frozen-floor Meta regression, real localhost three-hop missing-tail/retry tests. True multi-VM replica acceptance remains pending.

## D011 — Approved backend limit and isolated core validation

- Decision: retain the existing opaque full-snapshot CAS, configure etcd for 64 MiB requests and the client for 65 MiB envelopes, and defer the alternative snapshot paging draft. Deterministic map ordering removes insertion-order serialization differences without changing the stored logical schema.
- Reason: the user prioritized filesystem functionality and authorized a larger backend limit rather than blocking the main implementation on storage-model optimization.
- Validation strategy: core functional development and short performance diagnostics use memory-backed Meta first. Durable restart, backend parity and qualified persistent comparisons keep real backends.
- Evidence: native backup plus identical logical snapshot SHA across etcd restart; ordinary compaction/defrag; Linux library regression in the P3a candidate. Larger-than-4-MiB recovery, retention and full backend gates remain unverified.

## E002 — Runtime identity and harness setup corrections

- Redis short-slice `ca08025c...` identifies `/usr/bin/sudo`, not the Node executable. The staged Node was `cbf41733...`; that slice lacks a valid live Node `/proc/<pid>/exe` hash and cannot qualify current source identity. New launch scripts record the direct root child PID and hash its executable.
- The first P3a remount probe attempt used a build-VM-only share path on Node A; it did not exercise product behavior. Scripts were copied into the Linux guest and the attempt was retained.
- The STD-01 ext4 driver initially used a root-owned 0700 fixture, preventing pjdfstest privilege-drop traversal. Setting the harness fixture explicitly to 0755 fixed the three unexpected failures without changing suite expectations or exclusions. The full reference rerun passed 236 files / 8848 tests with 28 upstream TODO and zero unexpected failures.

## D012 — Authenticated batched Peer read authority

- Reason: the actual Node B DFS read failed with ENOSYS; receiver-side permission cannot trust predictable capability text or caller-provided version/range identity.
- Decision: Meta uses an OS-random per-process keyed BLAKE3 MAC over the canonical capability tuple. The authenticated receiver validates read grants in one batch on a bounded cache miss, then authorizes exact peer/epoch/copy/chunk/ranges. Cache hits never extend the original deadline; revocation/session rotation can remain cached for at most five seconds. Meta-unavailable misses fail closed. No file bytes pass through Meta.
- Cost: one Meta control RPC per uncached batch, zero per-range Meta RPC and no Meta request on a valid cache hit. Restart changes the signing secret; old uncached capabilities fail verification.
- Evidence: `evidence/afs-delivery/p3b-read-authority/`; Linux strict Clippy, 163 library tests/one explicitly ignored environmental test, Meta23/REST4/OwnerPeer2/VFS1, and actual localhost Meta-to-Peer gRPC read-stream authority tests. Node9c074d25/Meta938d3f1e immutable candidates precede subsequent namespace and special-inode changes. Full identified multi-VM read/fault validation remains pending.

## D013 — Namespace attributes and single configured root

- Problem: DFS create did not update parent timestamps; mkdir changed only link count. The virtual root always returned zero timestamps, even after namespace changes. Rename overwrote target nlink with zero, losing unrelated hardlinks.
- Design: namespace changes update parent mtime/ctime/revision and correct directory link deltas inside the same Meta CAS as the dentry. Rename touches source ctime, decrements only the replaced regular name, and same-inode aliases are a no-op. No additional Node RPC is added.
- Root: persist the existing root inode ID once with a distinct initialization operation, using CAS to resolve concurrent initialization. The existing GetInode API has no namespace argument and uses global root ID1; one configured DFS namespace is therefore enforced rather than silently aliasing multiple namespaces. General namespace expansion requires an explicit future interface design.
- Verification: focused source regressions added; merged Linux compile and real upstream rerun pending. The current deployed binary has not acquired these fixes.

## D014 — Combined Node installation and process identity

- Reason: starting separate OwnerFs and DFS Node processes with the same node ID fenced their sessions; unauthenticated default templates did not satisfy the real service authority boundary.
- Decision: one Node process hosts both independent mounts. Legacy role commands map to that process. Default local install creates TLS and trust using openssl; explicit cluster deployment supplies its intended topology/trust. Process identity includes executable, exact config, start ticks and command line before stop/status.
- Evidence: `evidence/afs-delivery/p1a/package-correction-20260930T081647Z/`; Linux negative selftests, checksum-verified package, real isolated installation/start/mount IO/stop and configuration/TLS preservation. Full DEP lifecycle/fault matrix remains pending.

## D015 — Namespace request identity and orphan rejection

- Independent review found namespace retries validated only the operation enum, allowing a reused operation ID with changed name/caller/arguments to return false success. New inode IDs also omitted caller scope.
- Decision: namespace request outcomes contain a deterministic whole-request digest and their existing result. Both early replay and CAS condition-failed replay validate that digest. Immutable inode identities hash length-prefixed caller ID plus operation ID. Removed directory parents and unlinked hardlink targets are rejected.
- Scope: existing Meta namespace methods and private outcome representation, with serde encoding on existing request types. No extra Node RPC, new backend, operation log or per-write version is introduced. Lease/data-commit outcomes retain their current representation.
- Compatibility: old persisted namespace outcomes without a semantic proof are rejected for retry; they are not trusted or silently rewritten. Existing inode identities continue to resolve, while newly created identities use the caller-scoped format. Coordinated binary upgrade is required for the new private outcome variant.
- Verification: independent scoped rereview reports no blockers after repairs; targeted Linux regressions and merged runtime rerun remain pending.

## E003 — Unaccepted host validation attempts

- DFS bounded-write agent attempted Rust formatting/check on macOS; host cargo check stopped at the Linux-only fuser guard and did not validate the code. An earlier host Python syntax check likewise is not Linux acceptance evidence.
- Correction: root's merged check, formatting, regressions and runtime probes execute on the Linux build/acceptance VMs. Only the Linux reruns can support milestone claims. Source edits are retained; no host test is counted as PASS.
- Build disk maintenance: with no AFS Cargo process active, root removed only the regenerable shared target/debug/incremental cache (~14GiB), retaining source, dependencies, evidence and immutable candidates. Free guest disk rose from4.2GiB to19GiB; next integrated validation uses CARGO_INCREMENTAL=0. The separate 3FS baseline Cargo target was not modified.

## D016 — Ordinary POSIX special inode and errno contracts

- Actual unchanged upstream smoke demonstrated missing FIFO/socket/device inode paths. Both backends use the shared existing FUSE module; DFS stores special inode metadata without Chunk or write lease, while OwnerFs creates it under the confined Home path through the existing OwnerFiles data service.
- Device creation uses the existing privileged uid0 caller model, checked rdev and parent write/search access. FIFO/socket support concerns the local mount namespace; no cross-node IPC guarantee is introduced.
- Raw Linux EFBIG/ELOOP and existing EBADF/ENAMETOOLONG keep stable coded identities through RPC and FUSE errno mapping. This fixes large-write overflow identity without changing replica or layout semantics.
- Evidence: Linux Meta29, library169/1ignored, error contracts4, strict Clippy and actual DFS original pjdfstest smoke241/241. OwnerFs still has15 hardlink alias failures and full product coverage is incomplete.

## E004 — Build snapshot and isolated helper corrections

- A root snapshot rsync copied historical source/evidence into two guest scratch directories and exhausted the build volume. Original host evidence was untouched. Removing only the duplicate guest evidence restored capacity; snapshots now exclude evidence/.local/.git/target/AppleDouble. 3FS's interrupted build must be retried with preserved progress; no baseline failure is attributed to source correctness.
- The isolated memory helper omitted an empty optional sudo argument on Meta stop. It stopped the old Node, then failed before stopping Meta. The argument was corrected, Meta identity rechecked and cleanup completed before v3 deployment. This is a harness failure, retained independently of product results.

## E005 — Source snapshot boundary correction

- A root snapshot selected the research parent `/workspace/dms` instead of product `/workspace/dms/source`, bringing12GiB historical caches into an isolated build scratch and exhausting disk. It failed before Cargo.toml; no product test was counted from that attempt.
- Removed only that failed scratch after confirming path and absent Cargo.toml. Retained source, baseline products, services and original evidence. Baseline build paused/resumed around recovery.
- Require product-root Cargo.toml preflight, cache/artifact/evidence exclusions and `set -e`; protobuf regeneration path is `common/protocol/proto`. Aggregate successful snapshot Meta30/library174 and Clippy evidence is separate from the failed attempt.

## D017 — Kernel privilege and timestamp cause preservation

- Problem: full memory POSIX validation exposed nonowner writes flattened into ordinary chmod and timestamp intent flattened into SystemTime. Broadening chmod would create an authorization hole.
- Decision: vendored fuser ABI7.33 exposes Linux killpriv v2 cause bits. Shared VFS gains compatible options-aware open/create/write/setattr methods; unimplemented privilege handling fails closed and capability advertisement is per backend. Owner/DFS and Peer forward the same cause without deriving it from daemon credentials. Timestamp NOW remains distinguishable from explicit timestamps.
- DFS: valid-write-lease commit/sync carries a default-false kill_suidgid flag; Meta only clears SUID and executable SGID. Both DataOnly and Full commits preserve this side effect. Old snapshots default to false; arbitrary mode setting remains an owner/root attribute operation.
- A killpriv commit also carries the frozen ctime, including DataOnly barriers; Meta rejects a missing ctime rather than inventing a different persisted timestamp. NOW updates use Meta's current time, not caller-selected numeric values. Focused request/replay regressions cover both rules.
- Scope: FUSE/vendor ABI, existing VFS/Peer/Meta interfaces and inline dirty attribute state; no new storage engine or transport plane. OwnerFs and DFS keep separate mounts and backend implementations. Backend option forwarding, serialization and actual standard-suite revalidation remain pending; staged code is not feature acceptance.

## D018 — Discover the actual RDMA source GID

- Problem: real A/B RXE file replication failed because the native endpoint assumed GID index0, which was IPv6 link-local and unreachable through the VM IPv4 network.
- Decision: enumerate the active port GID table; prefer a valid IPv4-mapped GID, retain valid IPv6 alternatives, and use the selected index for both advertised address and QP connection. Diagnose incompatible address families rather than silently falling back to gRPC.
- Scope: one native transport file; no new RPC or file/version/replica semantics. Physical hardware and multi-homed routing remain separately unqualified.
- Evidence: `evidence/afs-delivery/p3-rdma-cross/`: isolated identified memory-backed R2/M2 FUSE write/fsync/close and B replica recovery pass; separate R1/M1 empty-B-store FUSE peer read passes. Each transfers4194321RDMA bytes with0gRPC payload bytes. Original failure retained; no release or comparative performance claim.

## E006 — Terminal heartbeat errors and bounded build storage

- The old isolated v3 Node stopped after Meta rejected an expired session. Log gaps are consistent with VM/process suspension; this is an inference, not proof of the host cause. Retrying a terminal invalid session cannot restore authority. Node now stops on InvalidArgument/PermissionDenied/Unauthenticated registration failures while retaining bounded retries for transport outages; Linux classification regression passes.
- The shared build disk fell to1.1GiB free after debug artifacts accumulated. Only the old Redis target's3.9GiB reproducible incremental cache and host-hash-preserved duplicate RDMA binary archives were removed. Sources, evidence originals, baselines and the primary Cargo target remain. Feature checks subsequently left4.8GiB; further builds retain the4GiB floor.

## D019: preserve distributed advisory-lock identities

Problem: kernel-local advisory locks cannot enforce conflicting access across independent mounts. Vendored FUSE dispatch drops FLOCK flags and ignores INTERRUPT, preventing OFD/process-owner distinction and cancellable blocking waits.

Decision: preserve the existing fuser methods through options wrappers carrying raw lock flags, forward INTERRUPT with original unique identity, and add one shared inode-local advisory range core. POSIX owners use authenticated ingress session plus kernel lock owner; PID is report-only. Waiter ids also carry ingress session. POSIX lock release occurs on any fd close via flush owner, while flock release occurs at final OFD release. Range split/merge/EOF, cancellation, deadlock and resource capacity have regressions. Partial unlock capacity failure preserves original lock state; cancellation capacity fails closed rather than evicting pending interrupts.

Compatibility: existing vendor callbacks remain usable. No lock capability is advertised and no cross-mount behavior is claimed until backend, authenticated Peer control, cleanup and actual dual-mount tests are integrated. Authority stays Home for OwnerFs and inode owner for DFS; no Meta byte-range table. New focused file: `src/node/vfs/locks.rs`; shared data types remain in existing `types.rs`.

Evidence: Linux v9 vendor56 tests and product208 tests pass/2 explicit environment probes ignored. Initial vendor test-import failures and strict-lint findings remain in separate logs; corrected rerun is required before full integration. This is a source milestone, not a complete lock feature.

## E007: retain evidence while restoring VM capacity

ctl state volume became full during suite preparation. Historical p1b backups and completed LTP artifacts were archived and SHA-verified on host before duplicate guest removal. ctl now2.5GiB free; pinned source/etcd state and active product services preserved. The executable LTP install must be restored to a dedicated guest root ext4 tool path before product runs.

Build disk dropped below4GiB during independent builds. Identified v6/v7 binaries were copied to host and SHA-verified, then duplicate guest binaries removed. Old Redis-only `target/redis-p2e/debug/{deps,build,.fingerprint}` are reproducible build cache, not state/evidence; no process used that target. Removing only these restored approximately12GiB free. Original Linux logs, source snapshots and runtime data remain.

## D020 — Linux FUSE rename ABI isolation

- Problem: workspace all-features also enabled vendored fuser macfuse-4-compat; Linux FUSE_RENAME then parsed a16-byte argument instead of the kernel8-byte argument, consuming filenames and returning ENOSYS/incorrect names before Backend.
- Decision: gate macFUSE-only fields on both macOS target and feature; keep Linux ABI invariant. Add a wire regression for short and long filenames, run with all-features.
- Files: third_party/fuser/src/ll/fuse_abi.rs, ll/request.rs. Diagnostic parse warnings remain; rename callback trace is debug-level.
- Compatibility: no product public API, RPC or data-format change. Do not remove feature checks as a workaround.
- Evidence: v10 fresh mounts reproduce parse failure; Linux regression passes and v11 actual OwnerFs/DFS root rename passes. Full upstream suites remain pending.

## D021 — Authoritative distributed lock control and terminal errors

- Problem: a per-kernel lock table does not coordinate independent mounts. Lost set/release ACKs and cancellation arriving before registration require authenticated exact request identities; ordinary gRPC request timeouts must not cut off SETLKW waits.
- Scope: existing Peer control protocol/handlers, Home/inode-owner routing and existing Peer connection resources; no Meta byte-range lock state or data-plane lock RPC. Long waits use an uncached channel without the ordinary request timeout and a bounded dedicated server wait lane; unlock/cancel/release remain short control requests.
- Identity: ingress process session plus mount/kernel owner and exact waiter ID. Terminal Granted/Cancelled results are acknowledged only by a client that has received them. Unknown outcomes preserve identities. Retired request fences retain bounded capacity until scope cleanup; no unsafe arbitrary sequence watermark.
- Cleanup: session close must release its own locks without removing other live scopes, even when tombstone capacity is full. Remote cleanup failures retain exact targets and retry through existing maintenance; success ACK authorizes reclamation. Capacity blocks new admission, not targeted unlock/cleanup.
- Error catalog: IO_INTERRUPTED, IO_WOULD_BLOCK, IO_DEADLOCK and IO_NO_LOCKS preserve Linux EINTR/EAGAIN/EDEADLK/ENOLCK through wire and FUSE. Shared error package4, error contracts4, Meta35, OwnerPeer2 and REST4 pass on coherent Linux v19; library integration remains FAIL/TIMEOUT and is being repaired.
- Verification boundary: source regressions and actual independent A/B mounts both required. Current v11 deployed binary does not qualify these new handlers. Cross-probe accepts strict contention errno only; unsupported/capacity failures cannot masquerade as conflict. Full resource/lifecycle and release matrices remain unqualified.
