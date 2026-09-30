# Write Semantics

DFS separates ordinary visibility from recoverable durability. `write` changes the inode owner's dirty view. Explicit sync, synchronous write flags, close-time flush and background writeback can commit a recoverable file version. The default user contract follows JuiceFS: immediate visibility within one mount and close-to-open across mounts.

![Write timeline](../images/write-timeline.svg)

## User Contract

| Operation | Contract |
| --- | --- |
| `open` for read | Resolves a current file view; ordinary open is not a lifetime snapshot |
| `write` | Enters the inode owner's shared dirty state; other handles on the same mount, including previously opened read-only handles, can read accepted changes |
| read on the same write handle | Sees its own accepted writes through the owner dirty view |
| `fdatasync` | Commits file data, chunks, layout, length and inode head needed for recovery when there is dirty data |
| `fsync` | Includes `fdatasync`, then commits complete inode attributes such as `mtime` and `ctime` |
| `fsync(dir)` | Commits directory entries separately from file content |
| `flush` | Close-time error-reporting path: drains prior writes, satisfies the replica policy and commits data plus recovery-required metadata; repeated calls must be idempotent |
| `close` | Success makes completed writes durable under the configured policy and visible to subsequent opens on other mounts; a failure or timeout does not establish that guarantee |
| `release` | Releases the open-handle resources; it must not be the sole place to commit data or report commit failure |

After successful `close`, `fdatasync` or `fsync`, a subsequent `open` observes those completed changes or later committed changes. An already open reader on another mount is not promised immediate dirty visibility or a permanently pinned old version. Same-mount readers must observe local accepted writes through both the backend and FUSE/kernel caches.

A resolved committed read plan fixes one version, layout and chunk identity while selecting sources and assembling bytes. This internal coherence boundary is distinct from ordinary handle lifetime. If a barrier finds no dirty data or metadata, it can return without creating a new `FileVersion`.

The timeline shows a writer's `open`, `write`, same-mount `read`, and `close`, followed by a reader's new `open` on another mount. Node freezes and finalizes dirty data, obtains the required replicas, and asks Meta to commit before close returns. `V7` remains the committed head while dirty data is locally readable; `V8` becomes the recoverable head at Meta commit. Resource release follows the close-time flush; it is not the durability boundary.

## Sync Triggers

The user-visible triggers are POSIX barriers, close and open flags:

- `fdatasync(fd)` commits recoverable file data and the metadata required to find it again: chunks, layout, length and `head_version`.
- `fsync(fd)` does the same work and also commits complete inode attributes such as `mtime` and `ctime`. It still does not commit the parent directory entry; callers need `fsync(dir)` for that.
- `O_DSYNC` makes each accepted write wait for the data durability boundary before returning.
- `O_SYNC` makes each accepted write wait for the stronger file sync boundary before returning.
- `close(fd)` triggers FUSE flush of prior writes, including the data and metadata needed to reopen and recover them. Dup/fork can trigger multiple flushes before the final release; closing one descriptor must not discard another writer's state or create empty versions.
- Background writeback can freeze dirty state, finalize chunks and successfully commit a new `FileVersion`, including moving `head_version`. Applications cannot rely on when that happens; sync operations, sync write flags and successful close create caller-visible completion points.

## Node State

`InodeWriteState` is inode-owned, not handle-owned. It contains the base version, dirty extents, logical length, sequence numbers and pending error state. This lets multiple handles share one ordered current view.

All inode modifications are serialized by the inode owner. If a commit result is unknown, the precise request remains pending and later `write`, `resize` and `sync` operations for that inode are blocked until the result is recovered or failed. DFS does not start a next dirty batch in parallel behind an unknown commit.

## Commit Flow

```text
accepted writes
  -> InodeWriteState dirty ranges
  -> CommitBatch at a durability boundary
  -> ChunkObject creation and replica receipts
  -> LayoutRoot and FileVersion candidate
  -> Meta compare-and-set head_version
```

The file version number changes only at the Meta commit point. Chunks may already exist on nodes before the version is visible; without the Meta commit they are not the committed file head.


## Operation Uncertainty

A sync can reach a point where the node does not know whether Meta accepted the exact commit request. That result is not guessed. The inode keeps the operation identity as unresolved, returns or retries according to the caller contract, and blocks later `write`, `resize` and `sync` operations for that inode until recovery establishes whether the commit succeeded or failed.

Close-time flush follows the same uncertainty rule. Application timeout or descriptor release does not erase the pending request. Applications must check close errors; retrying close on a potentially released fd is unsafe. Reopen and validate the result or use application-level recovery when the outcome is unknown.

This rule prevents a later dirty batch from being ordered after an unknown version head. It also gives retry logic a precise idempotence key instead of relying on timeout interpretation.

## Definite Rejection And Metadata

Timeout, transport failure and uncertain store persistence retain the exact pending request. A confirmed Meta validation or condition rejection stops this owner state from accepting further modifications; recovery must resolve the lease and committed head before writing resumes. Reopening a file is not itself owner recovery.

`fdatasync` may leave complete timestamp attributes dirty after committing recoverable data. `fsync` must complete that metadata phase even when retrying an older data-only request. Timestamps describe the accepted mutation time, rather than the later sync time. An unchanged file does not require a new version merely to complete a barrier.
