# Replication

Replication is below the file layout layer. Layout code asks for durable chunks and receives `ChunkReceipt` records. It does not know whether a receipt came from the local fast path or a multi-node replication path.

![Replication](../images/replication.svg)

## Configurable Policy

Replica counts are configurable filesystem initialization policy, not hard-coded values.

```text
ReplicationConfig {
  desired_copies
  sync_required_copies
  min_distinct_nodes
  min_distinct_failure_domains
  local_copy
}
```

The base design treats this policy as stable for a filesystem instance. Per-inode dynamic policy revisions are intentionally outside the first contract.

## R=1 Local Fast Path

When the policy can be satisfied by one local durable copy, the writer finalizes the chunk locally and returns a receipt without peer data RPC.

## R=N Path

When the policy requires multiple synchronous copies, the writer derives a `ReplicationPlan` from Meta placement snapshots, sends the same chunk identity to peer targets, waits for enough `ReplicaAck` records and then returns a `ChunkReceipt`.

Each target still uses local chunk finalization: stage bytes, verify digest, make durable, publish, update local catalog and acknowledge.

## Evidence Records

| Record | Meaning |
| --- | --- |
| `ReplicaAck` | A target finalized one chunk copy on a specific device epoch and verified its digest |
| `ChunkReceipt` | The writer has enough acknowledgements to satisfy the synchronous policy |
| `CopyRecord` | Meta accepted a readable or recoverable copy into the catalog |
| `ReplicationTask` | Meta records async repair or extra-copy work |

Async repair can improve placement after a local-first success. It cannot retroactively make a weaker success mean a stronger synchronous durability contract.


## RPC Budget

The file layer batches work at sync boundaries, so the hot path is not one RPC per byte. The intended budget is:

| Case | Meta RPCs | Peer data RPCs | Notes |
| --- | --- | --- | --- |
| `write` with an existing owner | 0 | 0 | updates inode dirty state on the owner |
| R=1 `fdatasync` / `fsync` | commit RPC after local chunk receipts; placement may be cached or refreshed | 0 | local finalization satisfies the replica policy |
| R=N `fdatasync` / `fsync` | same final commit RPC after receipts | one transfer path per required remote copy or chain link | replication differs below `ChunkStore` only |
| read-only `open` | resolve current head and layout | 0 | fixes one version for the handle |
| fixed-version peer read | source lookup may be cached or refreshed | range read from selected peer | served only after authorization |

The exact transport can be request/response, streaming, RDMA descriptors or another data-plane mechanism. The contract is that small data can be carried inline, while large data should move through a data path that avoids unnecessary copies and still returns the same receipt semantics.
