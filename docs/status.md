# Implementation Status

Last updated: 2026-09-30.

This page is the only implementation-status page. Architecture pages describe the accepted target design.

The [delivery acceptance contract](acceptance.md) requires JuiceFS default same-mount visibility and close-to-open semantics. The dedicated environment, suite manifest, comparison baselines and acceptance Skill have not been prepared or executed. Existing framework checks are not proof that these release gates pass.

## Current Capability Matrix

| Area | Status | Evidence and limit |
| --- | --- | --- |
| Process foundation | Implemented foundation | `afs-meta`, `afs-node`, config, logging, metrics, REST, gRPC, FUSE wiring and shared errors exist. |
| FUSE module | Implemented foundation | Shared FUSE module supports separate OwnerFs and DFS sessions. |
| MetaStore | Experimental | memory, local-file and etcd-backed store code share one store interface. `MetaReadView` pins one acknowledged revision for compound source reads without requiring native multi-record transactions in the backend. Production HA and fencing are not complete. |
| OwnerFs | Experimental | Home-local files, P2P access to Home, root grants, handle checks and cleanup exist for small workspaces. |
| DFS R=1 write path | Experimental | DFS mount, shared inode dirty state, serialized mutation/commit, exact uncertain-request replay, truncate and sparse-file paths exist for local owner scenarios. Definite commit rejection blocks later mutations until owner recovery. Cross-node write owner routing is not complete. |
| FileVersion and layout model | Experimental | `FileVersion`, `LayoutRoot`, `Extent`, `ChunkObject`, CAS commit and base-chunk inheritance exist. Extent tree and compaction policy are not complete. |
| Replication | Framework / R=1 experimental | Replication config, placement snapshots, R1 path, receipt types and RN interfaces exist. Remote RN data movement and repair workers are not complete. |
| Local chunk engine | Experimental foundation | BLAKE3 identity, per-chunk files, local catalog, finalize and startup recovery exist. Pack backend, relocation, GC and full crash matrix are not complete. |
| File reads and default consistency | Framework / target gap | Current read-only handles pin the committed version, layout and length until close; writable handles read shared dirty state. This does not meet the target's immediate same-mount read-only visibility. Current DFS flush only observes errors and does not implement close-time commit. `DfsReadEngine`, local durable sources, per-operation grants, bounded peer batches, complete-response validation and a shared TLS/epoch/LRU pool exist; peer reads default to `DenyDfsReadAuthorizer`. |
| Native SDK | Foundation | `DfsLocalData` protocol and typed DFS client identity framework exist. The default Node service returns `UNIMPLEMENTED`; diagnostic `LocalData` remains separate and does not become OwnerFs. |
| RDMA | Transport probe foundation | RDMA lifecycle tests exist behind explicit environment requirements. File data path integration is not complete. |
| External spill | Not implemented | Design exists for `ExternalCommitted`; no product spill path is complete. |

## Known Open Items

- Close-time FUSE flush must commit prior writes and report commit errors; read-only view refresh and kernel cache handling must satisfy default same-mount visibility. Existing fixed-handle tests need to be reconciled with the acceptance contract.
- Redis persistent Meta backend and its single-Meta durability, recovery and inode-owner authority parity with etcd.
- OwnerFs REST location query exists, but its `status` is currently a constant `active`; it does not report Home availability. The existing OwnerFs acceptance runner checks creation and Meta-restart location, but the full `OPS-07` contract has not been validated.
- One-command release installation and process deployment acceptance; complete POSIX suites and MooseFS/3FS baselines.
- Cross-node DFS write owner routing.
- Full R=N replication, remote staging/finalize, repair workers and failure matrix.
- Extent tree, layout compaction and large-scale metadata cost validation.
- Directory `fsync(dir)`, rename/unlink/open-handle matrix, POSIX locks, xattr, ACL, `mmap` and broader compatibility.
- VerifiedCache, SeedLease, ExternalCommitted, spill recall and eviction gates.
- Read-grant protocol changes require a coordinated upgrade: request-level grant field 5 is reserved, each operation uses grant field 7, and `DfsReadGrant.caller_epoch` remains field 5.
- Existing persisted copy records need an explicit migration to the `CopyLocation` representation; no automatic old-format migration is provided.
- Metadata-only dirty attributes require explicit full `fsync`; background and drain policies do not yet provide a complete metadata-only recovery path.
- SHM/RDMA file-data path for DFS reads and writes.
- Meta instance election, active fencing and failover are deferred to the [post-acceptance TODO](acceptance.md#10-第一阶段验收后-todo); they are not first-stage delivery gates.
- Q1 whether every committed `FileVersion` must always use a `LayoutRoot`, and Q2 spill durability lower bound.
