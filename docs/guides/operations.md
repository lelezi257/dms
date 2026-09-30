# Operations

## Mounts

Run OwnerFs and DFS as separate mounts. Each mount has its own FUSE session, inode table, handle table and cache policy.

## Meta

The first-stage deployment runs one Meta process per filesystem. Stop and confirm termination of the old process before restarting it; do not run independent Meta instances against the same filesystem backend. The installer checks its own deployment's PID and port use. Meta election, automatic failover and fencing between instances require the separate [high-availability TODO](../acceptance.md#10-第一阶段验收后-todo).

The store interface supports alternate persistence backends. An etcd cluster's own election does not elect an AFS Meta leader.

## Workspace Location

Management tools query OwnerFs Home placement through `GET /v1/roots/{root_id}` on Meta. Node identity, root epoch and catalog revision describe placement; Home availability must be reported separately. Scheduling a workload on Home enables local access, while another node forwards operations to Home. This management path does not proxy file bytes or automatically migrate the workspace.

## Node Local Storage

Local chunk directories contain staged and finalized objects. Staged objects are not readable. Startup reconciliation must verify local records before serving them.

## Spill

External spill is outside the compute-cluster storage pool. Operators should monitor external write failures, orphan temporary objects and recall latency separately from local disk pressure.
