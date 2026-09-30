# Write Semantics

DFS separates ordinary visibility from recoverable durability. `write` changes the current dirty view on the inode owner. `fdatasync` or `fsync` commits a new recoverable file version.

![Write timeline](../images/write-timeline.svg)

## User Contract

| Operation | Contract |
| --- | --- |
| `open` for read | Fixes the current `FileVersion`, layout and length for that handle until close |
| `write` | Enters the inode owner's dirty state and is visible through that owner during normal operation |
| read on the same write handle | Sees its own accepted writes through the owner dirty view |
| `fdatasync` | Commits file data, chunks, layout, length and inode head needed for recovery when there is dirty data |
| `fsync` | Includes `fdatasync`, then commits complete inode attributes such as `mtime` and `ctime` |
| `fsync(dir)` | Commits directory entries separately from file content |
| `flush` | Drains frontend work and reports known errors; it is not a durability boundary |
| `close` | Releases the handle; it does not commit data and does not provide close-to-open consistency |

After `fsync` succeeds, a new `open` observes that committed version or a later version. An already open read-only handle keeps its fixed version until close. If a sync operation finds no dirty data or metadata, it can return without creating a new `FileVersion`.

## Sync Triggers

The user-visible triggers are explicit POSIX barriers and open flags:

- `fdatasync(fd)` commits recoverable file data and the metadata required to find it again: chunks, layout, length and `head_version`.
- `fsync(fd)` does the same work and also commits complete inode attributes such as `mtime` and `ctime`. It still does not commit the parent directory entry; callers need `fsync(dir)` for that.
- `O_DSYNC` makes each accepted write wait for the data durability boundary before returning.
- `O_SYNC` makes each accepted write wait for the stronger file sync boundary before returning.
- Background writeback can freeze dirty state, finalize chunks and successfully commit a new `FileVersion`, including moving `head_version`. Applications cannot rely on when that happens; only explicit sync operations and sync write flags create a caller-visible durability completion point.

## Node State

`InodeWriteState` is inode-owned, not handle-owned. It contains the base version, dirty extents, logical length, sequence numbers and pending error state. This lets multiple handles share one ordered current view.

All inode modifications are serialized by the inode owner. If a commit result is unknown, the precise request remains pending and later `write`, `resize` and `sync` operations for that inode are blocked until the result is recovered or failed. DFS does not start a next dirty batch in parallel behind an unknown commit.

## Commit Flow

```text
accepted writes
  -> InodeWriteState dirty ranges
  -> CommitBatch at sync boundary
  -> ChunkObject creation and replica receipts
  -> LayoutRoot and FileVersion candidate
  -> Meta compare-and-set head_version
```

The file version number changes only at the Meta commit point. Chunks may already exist on nodes before the version is visible; without the Meta commit they are not the committed file head.


## Operation Uncertainty

A sync can reach a point where the node does not know whether Meta accepted the exact commit request. That result is not guessed. The inode keeps the operation identity as unresolved, returns or retries according to the caller contract, and blocks later `write`, `resize` and `sync` operations for that inode until recovery establishes whether the commit succeeded or failed.

This rule prevents a later dirty batch from being ordered after an unknown version head. It also gives retry logic a precise idempotence key instead of relying on timeout interpretation.
