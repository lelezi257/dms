# Architecture

AFS separates user-visible file semantics from the movement of file bytes. Applications use mounts or a DFS-specific SDK. `afs-node` runs close to workloads and moves bytes. `afs-meta` owns namespace, leases, file versions, placement and copy catalog state.

![AFS architecture](images/overview.svg)

## First Principles

1. **Meta owns authority, not data bytes.** Meta commits namespace, inode, version, layout, placement and copy records. Steady-state file bytes move between nodes, local disks, verified cache and optional spill.
2. **DFS stores committed data as immutable chunks.** Mutable writes accumulate in inode dirty state. An ordered commit moves `InodeRecord.head_version` to a new immutable `FileVersion`; explicit sync, synchronous write flags, close-time flush and background policies can trigger that commit. Ordinary writes do not create a version per request.
3. **Reads resolve a coherent view before selecting sources.** A committed read plan fixes `FileVersionId`, layout and length while assembling bytes. Same-mount readers also observe local accepted dirty writes. Ordinary open is not a lifetime snapshot; the cross-mount contract is close-to-open.
4. **Replication lives below file layout.** The layout layer asks for durable chunks and receives receipts. R=1 and R=N split below `ChunkStore` and rejoin before Meta commits a new version.
5. **OwnerFs and DFS are separate mounts.** They share process and FUSE infrastructure but have separate backend state machines and cache policies.

## Entry Points

| Entry point | Backend | Notes |
| --- | --- | --- |
| FUSE mount | OwnerFs or DFS | Separate mount sessions for separate backends |
| DFS SDK | DFS only | Intrusive high-performance path over DFS semantics; it is not a generic OwnerFs API |
| Future block adapter | DFS | Uses fixed-version range reads for base images and normal DFS writes for new data |

## Component Responsibilities

| Component | Responsibility |
| --- | --- |
| `afs-meta` | Namespace, inode records, write leases, file versions, layout roots, placement snapshots, copy catalog, idempotent commit results |
| `afs-node` | FUSE sessions, DFS dirty state, OwnerFs Home access, chunk storage, replication execution, peer reads, verified cache, spill workers |
| Local storage | Staged bytes, digest verification, durable publish, local catalog and reader pins |
| Peer transport | Node-to-node control, replication transfer and fixed-version range reads |
| External spill | Optional capacity and cold source after write, verification and Meta commit |

Detailed mechanisms are in the [architecture pages](README.md#mechanisms).

The [delivery acceptance scope](acceptance.md) requires FUSE, OwnerFs/DFS, etcd/Redis and RDMA. SDK, verified cache and spill describe the broader design and are outside this release.


## Detailed Contract Map

| Contract area | Page |
| --- | --- |
| Object model, UML fields, sparse files and `LayoutRoot` | [Data Model](architecture/data-model.md) |
| Write visibility, sync triggers, `O_SYNC`, `O_DSYNC` and uncertain commits | [Write Semantics](architecture/write-semantics.md) |
| Replica policy, R=1/R=N split and RPC budget | [Replication](architecture/replication.md) |
| Chunk finalization, layout COW, physical COW and recovery | [Local Storage And COW](architecture/local-storage.md) |
| Fixed-version reads, `CopyState`, seed leases, eviction and spill | [Read, Cache And Spill](architecture/read-cache-spill.md) |
| OwnerFs Home placement and failure boundary | [OwnerFs](architecture/ownerfs.md) |
| Meta commit authority, read views and backend transaction boundary | [Meta And Transactions](architecture/meta.md) |
