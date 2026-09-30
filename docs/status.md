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
| DFS R=1 write path | Experimental | DFS mount, shared inode dirty state, serialized mutation/commit, exact uncertain-request replay, truncate and sparse-file paths exist for local owner scenarios. Definite commit rejection blocks later mutations until owner recovery. Identified A/B memory-backed mounts pass short remote-owner writes, handleless resize, same-mount dirty reads and owner-handover scenarios. Complete fault and backend coverage remains unqualified. |
| FileVersion and layout model | Experimental | `FileVersion`, `LayoutRoot`, `Extent`, `ChunkObject`, CAS commit and base-chunk inheritance exist. Extent tree and compaction policy are not complete. |
| Replication | Experimental gRPC chain | Configurable replica targets/synchronous minimum, global placement, authenticated receiver write grants, bounded staging and ordered durable chain acknowledgements exist. Real localhost missing-tail/exact-retry tests pass; full multi-VM faults and repair workers remain incomplete; RDMA payload adapters have short proof only. |
| Local chunk engine | Experimental foundation | BLAKE3 identity, per-chunk files, local catalog, finalize and startup recovery exist. Pack backend, relocation, GC and full crash matrix are not complete. |
| File reads and default consistency | Local experimental | Ordinary readonly handles share the local inode dirty view; each read pairs one base version/layout. Writable close flush commits recovery state, retains uncertain requests and reports errors. DFS cached write-through passes a real Linux warm readonly/MAP_SHARED-read/overwrite/append/shrink-grow slice. Remote owner forwarding of data and attributes passes the identified eight-case A/B short probe, including write-only providers and retained-state handover. Full concurrency and permission matrices remain incomplete. `DfsReadEngine`, bounded peer batches and connection pool exist; authenticated batched Meta read-grant validation and bounded receiver caches pass Linux source and localhost streaming regressions; the identified A/B handover and peer-read short probe passes. Full security and fault matrices remain incomplete. |
| Native SDK | Foundation | `DfsLocalData` protocol and typed DFS client identity framework exist. The default Node service returns `UNIMPLEMENTED`; diagnostic `LocalData` remains separate and does not become OwnerFs. |
| RDMA | Experimental product adapters | An identified Linux RXE product-adapter probe verifies two4MiB durable replica transfers and75000B peer reads with zero gRPC file payload, exact content/retry and denied forged authority. Actual cross-VM RXE FUSE R2 synchronous writes and R1 peer reads each verify4194321 bytes with zero gRPC file payload; the B replica survives restart. Complete fallback, fault and security matrices remain unverified. |
| External spill | Not implemented | Design exists for `ExternalCommitted`; no product spill path is complete. |

## Current Validation Checkpoint

Linux v37 passes formatting, strict workspace/all-target/all-feature Clippy, 291 library tests (two explicitly ignored environmental probes), 57 interface contract tests, four shared-error tests, five privileged real FUSE tests, feature checks and Node/Meta build. All 143 captured compile inputs match the local source. Raw source and runtime evidence is [versioned here](../development/evidence/20261001-owner-rename/README.md).

Identified A/B v37 memory-backed OwnerFs/DFS mounts pass ten cross-mount consistency scenarios. These include same-mount visibility, close-to-open, remote owner writes, local and remote-Home overwrite rename with a surviving hardlink, owner handover, retained local state and an existing write-only remote handle surviving handleless resize. Assertions do not poll until stale data disappears. [Earlier v36 evidence](../development/evidence/20261001-authority/README.md) separately records seven-step OwnerFs/DFS cross-mount lock checks with 35-second waits, a two-step exact-target 18-second Meta pause and 500 DFS/ext4 random operations under a 180-second functional bound. These earlier runs retain their own binary identities.

The complete v37 OwnerFs pjdfstest run executes all 236 files and accounts for 8819 TAP checks, with zero unexpected failures/skips and 28 upstream TODO. Before/after process, configuration and mount identity checks pass. The preserved v36 full run has ten unexpected `rename/23.t` failures: the surviving hardlink returned `ESTALE`. Two targeted regressions fail with the original rename logic and pass after canonical path rebind. The current DFS full run remains pending; [the earlier DFS v26 full result](../development/evidence/20260930-resume/README.md) binds its own binary and does not qualify the current candidate or unrun backend variants. Full-suite success here covers this development matrix, not the entire POSIX release contract.

The formal 69-case release manifest remains NOT_RUN and the environment lock PREPARING. No mandatory release or performance gate is declared complete by these short checks.

## Known Open Items

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
