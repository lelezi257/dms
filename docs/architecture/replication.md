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

## Asynchronous Repair

With desired copies N and synchronous minimum M, a file barrier commits after M
durable copies. Meta records the remaining work as a `ReplicationTask` in the
same file commit. The file's version and extent layout do not change when repair
adds copies.

1. A Node holding a live durable source asks Meta for a task. Meta records a
   fenced `ReplicationClaim` containing the exact source, chunk, worker session,
   device identities and source-first target chain.
2. The worker verifies the local immutable chunk and transfers it through the
   existing replica data path. Receivers validate the exact claim before
   persisting; gRPC and RDMA carry the same authority and receipt semantics.
3. The worker reports the complete target receipts. Meta atomically records the
   copies and completes the task. Unreported physical copies are not treated as
   published replicas.

Unknown claim or report results retain the original request identity. Retrying
a report does not retransmit data. An expired lease prevents starting a new
transfer; an exact report for the still-current claim can complete after its
deadline. Once a claim is replaced, its reports cannot promote copies. Generic
CAS conflicts keep the pending report; only an explicit superseded result ends
that claim.

Unavailable nodes and confirmed corrupt copies are distinct. No live source
means `BlockedNoSource`, not confirmed permanent loss. Recovered devices can
serve their retained copies after the current session and catalog evidence are
validated. A Node polling for repair detects lost availability and reactivates
repair work; this is independent of an application's next read.

`GET /v1/dfs/chunks/{chunk_id}/replication` reports current available copies,
placement and tasks from one Meta read view. It checks backend health and never
infers permanent loss solely from unavailable nodes.


## RPC Budget

The file layer batches work at durability boundaries, including close-time flush, so the hot path is not one RPC per byte. The intended budget is:

| Case | Meta RPCs | Peer data RPCs | Notes |
| --- | --- | --- | --- |
| local-owner `write` | 0 | 0 | updates inode dirty state; remote-owner access adds one forwarding RPC |
| R=1 changed-data sync | 1 final commit | 0 | cached placement and valid lease; refresh/renew may add RPCs |
| M synchronous copies | 1 final commit, plus M-1 receiver-authority checks per chunk | M-1 chain-link transfers per chunk | placement refresh/lease renewal are additional control calls |
| repair to N copies | 1 claim + N-1 receiver-authority checks + 1 report per chunk | N-1 chain-link transfers | source-first full chain; empty polls and retries add maintenance calls |
| read-only `open` | head lookup + version/layout lookup, up to 2 without cache | 0 | resolves the current view; does not create a lifetime snapshot |
| fixed-version peer read | 0 on valid source cache; source refresh otherwise | 1 range-read batch per selected peer/context | each operation retains its authorization |

The exact transport can be request/response, streaming, RDMA descriptors or another data-plane mechanism. The contract is that small data can be carried inline, while large data should move through a data path that avoids unnecessary copies and still returns the same receipt semantics.


## Peer Connection Reuse

Peer connection pooling belongs below replication and read planning. A pool can reuse channels and cap peer endpoints, but it must not cache authorization or file-version decisions. Those decisions stay with the `ReplicationPlan`, `DfsReadGrant` and per-operation request identity.

A no-change barrier can need zero Meta commits. A metadata-only full sync needs one metadata commit. Full sync following an unresolved data-only request may need the original request retry and then a metadata commit. These counts describe logical successful requests; transport retries reuse the exact operation identity and add attempts. Node-to-Meta control requests are separate from Node-to-Node byte transfers.
