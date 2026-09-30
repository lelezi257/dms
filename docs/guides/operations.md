# Operations

## Mounts

Run OwnerFs and DFS as separate mounts. Each mount has its own FUSE session, inode table, handle table and cache policy.

## Meta

Deployments must provide a single active Meta authority. The store interface can use different persistence backends, but it does not by itself create safe active-active Meta service.

## Node Local Storage

Local chunk directories contain staged and finalized objects. Staged objects are not readable. Startup reconciliation must verify local records before serving them.

## Spill

External spill is outside the compute-cluster storage pool. Operators should monitor external write failures, orphan temporary objects and recall latency separately from local disk pressure.
