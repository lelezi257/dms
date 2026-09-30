# Implementation Status

Last updated: 2026-09-30.

This page is the only implementation-status page. Architecture pages describe the accepted target design.

## Current Capability Matrix

| Area | Status | Evidence and limit |
| --- | --- | --- |
| Process foundation | Implemented foundation | `afs-meta`, `afs-node`, config, logging, metrics, REST, gRPC, FUSE wiring and shared errors exist. |
| FUSE module | Implemented foundation | Shared FUSE module supports separate OwnerFs and DFS sessions. |
| MetaStore | Experimental | memory, local-file and etcd-backed store code share one store interface. MetaReadView can pin one committed snapshot for compound source reads without requiring native multi-record transactions in the backend. Production HA and fencing are not complete. |
| OwnerFs | Experimental | Home-local files, P2P access to Home, root grants, handle checks and cleanup exist for small workspaces. |
| DFS R=1 write path | Experimental | DFS mount, inode dirty state, file-version commit, truncate and sparse-file paths exist for local owner scenarios. Cross-node write owner routing is not complete. |
| FileVersion and layout model | Experimental | `FileVersion`, `LayoutRoot`, `Extent`, `ChunkObject`, CAS commit and base-chunk inheritance exist. Extent tree and compaction policy are not complete. |
| Replication | Framework / R=1 experimental | Replication config, placement snapshots, R1 path, receipt types and RN interfaces exist. Remote RN data movement and repair workers are not complete. |
| Local chunk engine | Experimental foundation | BLAKE3 identity, per-chunk files, local catalog, finalize and startup recovery exist. Pack backend, relocation, GC and full crash matrix are not complete. |
| Fixed-version reads | Framework / local durable read experimental | `DfsReadEngine`, local durable source, Meta source query shape, gRPC peer range-read code, explicit `PeerAuthenticator` and `DfsReadAuthorizer` seams exist. Node currently uses a deny authorizer until real read grants are connected. VerifiedCache, SeedLease, Spill and production connection management are not complete. |
| Native SDK | Foundation | `DfsLocalData` protocol and typed DFS client framework exist. The default Node service returns `UNIMPLEMENTED`; diagnostic `LocalData` remains separate and does not become OwnerFs. |
| RDMA | Transport probe foundation | RDMA lifecycle tests exist behind explicit environment requirements. File data path integration is not complete. |
| External spill | Not implemented | Design exists for `ExternalCommitted`; no product spill path is complete. |

## Known Open Items

- Cross-node DFS write owner routing.
- Full R=N replication, remote staging/finalize, repair workers and failure matrix.
- Extent tree, layout compaction and large-scale metadata cost validation.
- Directory `fsync(dir)`, rename/unlink/open-handle matrix, POSIX locks, xattr, ACL, `mmap` and broader compatibility.
- VerifiedCache, SeedLease, ExternalCommitted, spill recall and eviction gates.
- SHM/RDMA file-data path for DFS reads and writes.
- Meta active fencing, failover and production HA.
- Q1 whether every committed `FileVersion` must always use a `LayoutRoot`, and Q2 spill durability lower bound.
