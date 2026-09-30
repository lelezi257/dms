# AFS Architecture Principles

This page defines stable product and architecture principles. Implementation progress is recorded only in [Implementation Status](docs/status.md).

## POSIX First

AFS exposes a shared file namespace through POSIX-compatible interfaces. Applications use normal directories, files, reads, writes and sync operations. Default consistency follows JuiceFS: immediate visibility within a mount and close-to-open across mounts. See the [visibility contract](docs/architecture/write-semantics.md).

High-performance SDKs are additional DFS entry points over the same namespace and file semantics.

## DistributedFs Is The General Path

`DistributedFs` is the general distributed filesystem backend. It owns file layout, chunks, replicas, reads, repair, cache, capacity management and optional spill.

Images, snapshots and checkpoints are important optimization workloads, but they are not a separate Blob filesystem. They are stable file versions in the same DFS model.

## OwnerFs Is A Small-Cluster Workspace Path

`OwnerFs` is specialized for 1 to 4 node Agent workspaces. A workspace has a Home node. The Home stores bytes as normal local files. Workloads on the Home use local files; workloads on other nodes reach the Home through P2P.

OwnerFs and DistributedFs are separate mounts with separate runtime inode tables, handle tables, cache policy and data layout. They share FUSE module code and common transport utilities.

## Immutable Chunks Form The DFS Base

A committed `ChunkObject` is immutable. A file remains mutable because its current dirty view can change before sync, and because its `InodeRecord.head_version` can move from one immutable `FileVersion` to the next.

## FileVersion Is The Read Consistency Boundary

A resolved committed read plan fixes a `FileVersion`, layout and length while assembling data from sources. Ordinary read-only open is not a lifetime snapshot. Data from different nodes can be combined only when it belongs to the same resolved version and passes chunk identity checks.

Replica location, cache location and external location can change without changing the file version or chunk content identity.

## Sync Commits File State, Not Business Publication

`fdatasync` commits file data and recovery-required metadata. `fsync` includes that work and also syncs complete inode attributes such as `mtime` and `ctime`.

File sync does not imply parent directory sync. Directory entries require `fsync(dir)`. Successful close flushes prior writes and commits their recoverable state; release only cleans up resources. These barriers do not create a business snapshot, alias or pin.

## Replication Lives Below ChunkStore

File layout code consumes `ChunkReceipt` records. Single-replica and multi-replica paths split below `ChunkStore::put_batch` and rejoin before FileVersion commit.

Replica count is a filesystem initialization policy. It is not encoded into each file version or extent.

## Local Disks Are The Near-Compute Pool

Compute nodes can contribute SSD, NVMe, HDD or other local disks. Relative to external object storage, these disks form the near-compute storage layer. Inside AFS, durable replicas, verified cache and external committed copies remain distinct roles.

## Object Storage Is Optional Spill

AFS can run with cluster local disks only. External object storage is an optional layer for spill, cold data, archive or disaster recovery.

Local data can be evicted because of an external copy only after external write, verification and metadata commit have all completed.
