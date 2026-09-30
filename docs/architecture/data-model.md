# Data Model

DFS exposes mutable files but stores committed content as immutable versions and chunks.

![Data model](../images/data-model.svg)

## Example: `/xxx.txt`

A user creates `/xxx.txt`, writes data and calls `fsync`. Meta holds a stable inode. Node turns the synced byte ranges into chunks. Meta commits a new file version and moves the inode head to it.

A later reader opens `/xxx.txt`, fixes the current head version, loads the layout for that version and reads the referenced chunks. If another writer commits a newer version while the reader is open, the existing read-only handle keeps its fixed version, layout and length until close.

## Core Objects

| Object | Role |
| --- | --- |
| `Dentry` | Maps a directory name to an inode |
| `InodeRecord` | Stable identity, attributes and mutable `head_version` pointer |
| `FileVersion` | Immutable committed file view at one length |
| `LayoutRoot` | Root of the layout for one file version |
| `Extent` | Maps a file range to a chunk range |
| `ChunkObject` | Immutable content identity: length, digest and encoding |
| `CopyRecord` | Cluster-visible record of where a chunk copy can be read or recovered |

Only `InodeRecord.head_version` moves when a committed file changes. Existing versions, layouts and chunks remain valid until lifecycle rules allow collection.

## Holes And Sparse Files

`FileVersion.length` defines EOF. Extents describe data ranges. A range inside `[0, length)` without an extent is a hole and reads as zero. Growing a file can create holes without writing zero chunks.

## LayoutRoot Boundary

`LayoutRoot` is the named root of a committed version's layout. It can point to inline extents, a tree, or another layout representation. Readers only depend on the resolved mapping, not on the physical representation.

Open question Q1: whether every committed `FileVersion` must always have a `LayoutRoot`, including tiny files that could otherwise inline all extents directly in the version record. The status page tracks implementation progress; this page defines the contract.

## UML View

```text
Dentry {
  parent_inode: InodeId
  name: String
  target_inode: InodeId
  entry_version: u64
}

InodeRecord {
  inode: InodeId
  kind: File | Directory | Symlink
  attrs: InodeAttrs
  head_version: Option<FileVersionId>
  write_owner: Option<NodeEpoch>
}

FileVersion {
  id: FileVersionId
  inode: InodeId
  parent: Option<FileVersionId>
  length: u64
  layout: LayoutRoot
  created_by: OperationId
}

LayoutRoot {
  version: FileVersionId
  extents: InlineExtents | ExtentTreeRoot
}

Extent {
  file_offset: u64
  length: u64
  chunk: ChunkId
  chunk_offset: u32
}

ChunkObject {
  id: ChunkId
  length: u32
  digest: Digest
  encoding: Encoding
}

CopyRecord {
  chunk: ChunkId
  location: CopyLocation
  state: CopyState
  epoch: DeviceEpoch
}
```
