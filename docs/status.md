# Implementation Status

Last updated: 2026-09-30.

This page is the only implementation-status page. Architecture pages describe the accepted target design.

The [delivery acceptance contract](acceptance.md) requires JuiceFS default same-mount visibility and close-to-open semantics. A dedicated Linux environment, suite manifest, runner and acceptance Skill are prepared for development; reference suites and comparator prerequisites are still incomplete. Short regressions are not proof that release gates pass.

## Current Capability Matrix

| Area | Status | Evidence and limit |
| --- | --- | --- |
| Process foundation | Implemented foundation | `afs-meta`, `afs-node`, config, logging, metrics, REST, gRPC, FUSE wiring and shared errors exist. |
| FUSE module | Implemented foundation | Shared FUSE module supports separate OwnerFs and DFS sessions. |
| MetaStore | Experimental | memory, local-file, etcd and Redis share one store interface. Redis uses atomic full-snapshot CAS and checks AOF always/noeviction/no TTL; real Redis CAS, configuration rejection and AOF restart short regressions pass; actual Meta-process recovery/fault parity remains unverified. `MetaReadView` pins one acknowledged revision without requiring native multi-record backend transactions. Meta HA is deferred. |
| OwnerFs | Experimental | Home-local files, P2P access to Home, root grants, handle checks and cleanup exist. Dirty local close flush performs data sync; remote flush reaches the Home boundary. Identified Linux TLS/etcd-backed local and remote-Home mounts pass 11 POSIX and 4 multi-user short checks. Ordinary test fixtures live inside a workspace; deleting the workspace authority root is not implemented as ordinary rmdir. Full fault/POSIX coverage remains incomplete. |
| DFS R=1 write path | Experimental | DFS mount, shared inode dirty state, serialized mutation/commit, exact uncertain-request replay, truncate and sparse-file paths exist for local owner scenarios. Definite commit rejection blocks later mutations until owner recovery. Remote-owner operation interfaces exist; their actual multi-mount behavior and fault coverage remain unqualified. |
| FileVersion and layout model | Experimental | `FileVersion`, `LayoutRoot`, `Extent`, `ChunkObject`, CAS commit and base-chunk inheritance exist. Extent tree and compaction policy are not complete. |
| Replication | Experimental gRPC chain | Configurable replica targets/synchronous minimum, global placement, authenticated receiver write grants, bounded staging and ordered durable chain acknowledgements exist. Real localhost missing-tail/exact-retry tests pass; full multi-VM faults and repair workers remain incomplete; RDMA payload adapters have short proof only. |
| Local chunk engine | Experimental foundation | BLAKE3 identity, per-chunk files, local catalog, finalize and startup recovery exist. Pack backend, relocation, GC and full crash matrix are not complete. |
| File reads and default consistency | Local experimental | Ordinary readonly handles share the local inode dirty view; each read pairs one base version/layout. Writable close flush commits recovery state, retains uncertain requests and reports errors. DFS cached write-through passes a real Linux warm readonly/MAP_SHARED-read/overwrite/append/shrink-grow slice. Remote owner forwarding/cache, full concurrency and permission matrices remain incomplete. `DfsReadEngine`, bounded peer batches and connection pool exist; authenticated batched Meta read-grant validation and bounded receiver caches pass Linux source and localhost streaming regressions; the identified merged multi-VM candidate rerun remains pending. |
| Native SDK | Foundation | `DfsLocalData` protocol and typed DFS client identity framework exist. The default Node service returns `UNIMPLEMENTED`; diagnostic `LocalData` remains separate and does not become OwnerFs. |
| RDMA | Experimental product adapters | An identified Linux RXE product-adapter probe verifies two4MiB durable replica transfers and75000B peer reads with zero gRPC file payload, exact content/retry and denied forged authority. Actual cross-VM RXE FUSE R2 synchronous writes and R1 peer reads each verify4194321 bytes with zero gRPC file payload; the B replica survives restart. Complete fallback, fault and security matrices remain unverified. |
| External spill | Not implemented | Design exists for `ExternalCommitted`; no product spill path is complete. |

## Current Validation Checkpoint

The [delivery handoff](handoff.md) identifies the current source candidate, runnable old lane and portable continuation inputs. Linux v25 passes formatting, strict workspace/all-target/all-feature Clippy, 258 library tests (two explicitly ignored environmental probes), 53 interface contract tests, four shared-error tests and Node/Meta build. The portable runner/probe inputs pass 77 Linux selftests. Raw checkpoint logs are [versioned here](../development/evidence/20260930-checkpoint/README.md).

The actual v11 DFS mount completed the full pjdfstest suite: 236 files, 8819 checks, zero unexpected failures and 28 TODO. OwnerFs completed the same suite with two unexpected failures (`ftruncate/00.t` and `unlink/14.t`). Both defects are repaired in source; deployment and upstream rerun of the new candidate remain pending. v11 runtime evidence does not qualify v25.

The formal 69-case release manifest remains NOT_RUN and the environment lock PREPARING. No mandatory release or performance gate is declared complete by these short checks.

## Known Open Items

- Deploy the identified merged candidate and validate cross-mount fcntl/flock, cancellation, delayed waits and close/session cleanup on actual OwnerFs/DFS mounts. Capacity, precise retired identity and cleanup retry have source regressions; long-run behavior remains unqualified.
- Rerun OwnerFs upstream failures and full applicable POSIX matrices without adding exclusions; complete remote-owner consistency, permissions, namespace, mmap, xattr, ACL and directory durability coverage.
- Complete cross-node DFS file-operation and lock authority, stale owner rejection and uncertain-result fault verification; preserve exact pending identity and inode serialization.
- Finish R=N multi-VM faults, repair workers, placement and source-loss behavior, full RDMA/fallback/security/lifetime tests and resource accounting.
- Core development uses memory Meta. Complete etcd/Redis persistence, parity and recovery separately after core functionality/performance development. Existing etcd integration uses an approved enlarged request limit; full-snapshot retention and serialization cost remain open.
- Complete local chunk crash/reconcile/GC, pack/relocation, compaction and large-file bounded-memory/capacity/corruption cases.
- Validate actual Home backend availability, full OPS diagnostics/readiness/backpressure and DEP clean/offline/idempotent installation, shutdown and restart matrices.
- Qualify fair MooseFS/3FS baselines before performance tuning and paired performance claims. Stock MooseFS strong-durability matching and the ARM64 patched 3FS reference remain open prerequisites.
- Run final 8 GiB, full FSx/random-operation seeds and eight-hour soak; source or short-mount regression success is insufficient.
- Read-grant protocol changes require coordinated upgrade: request-level grant field 5 is reserved, each operation uses grant field 7, and `DfsReadGrant.caller_epoch` remains field 5. Existing persisted copy records require explicit migration to `CopyLocation`; no automatic old-format migration is provided.
- DFS SDK, product cache/spill and Meta election/HA remain outside the first-stage gates, as recorded in the [post-acceptance TODO](acceptance.md#10-第一阶段验收后-todo).
