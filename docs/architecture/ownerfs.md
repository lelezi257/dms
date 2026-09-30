# OwnerFs

OwnerFs is a small-cluster workspace backend. It keeps a workspace on a Home node and forwards remote operations back to Home when compute moves away.

![OwnerFs](../images/ownerfs.svg)

## Home Node

The Home stores workspace bytes as ordinary local files. Local Agent work uses the shortest path. Remote compute uses peer operations to reach the Home, which keeps authoritative file handles and validates root, peer session and authorization.

## Scope

OwnerFs optimizes:

- frequent small file operations in one Agent workspace;
- 1 to 4 node deployments;
- cases where the main Agent and workspace can stay colocated;
- predictable peer forwarding when a worker runs away from Home.

OwnerFs does not implement the DFS chunk, replica, cache or spill state machine. It shares FUSE module code and common transport utilities with DFS, but its backend state is separate.

## Failure Boundary

OwnerFs durability is the Home node local filesystem durability plus the Meta state that grants and tracks the workspace. Remote peer access is a transport path to Home, not an extra durable copy. If Home fails permanently, recovery depends on the deployment's local disk and Meta recovery policy.

There is no automatic OwnerFs-to-DFS snapshot conversion in the base design.
