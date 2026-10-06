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

## Remote Data Transport

The Node chooses the transport for remote Home reads and writes. File lookup, open, sync and close remain authenticated RPC commands. With RDMA, `OwnerNegotiateData` and `OwnerCloseData` on NodeControl manage a bounded registered buffer; OwnerFiles carries the file handle, offset, length and buffer descriptor. File bytes move through RDMA READ or WRITE rather than inline protobuf data.

Owner transport sessions use a separate registry and bind the authenticated peer to the exact root grant, caller process session, Home process session and fence. A transport session does not grant file access. Each operation validates file rights and the open handle; a write validates them before pulling bytes and again before modifying the file. Payload checksums are verified before accepting transferred contents. Short reads, EOF and partial writes retain their file semantics.

`rdma` requires an available RDMA device and never retries an uncertain data operation through gRPC. `auto` can select gRPC for an explicit transport absence before issuing the file operation. Authorization, protocol, checksum and unknown write-result errors are not fallback reasons. RDMA reads disable open-time inline prefetch so file data uses the selected transport.

Closing a transport session removes its admission entry. Work already admitted retains its endpoint until completion; cancellation cannot release or reuse a buffer still owned by an in-flight transfer. Authenticated exact-scope close remains possible after grant revocation, without granting further file access.

Client admission is shared by the Node's peer clients and limits concurrent registered windows before allocation. The permit remains attached to the endpoint through blocking work and teardown. Exhausted admission returns a capacity error; it does not authorize a transport fallback. Only gRPC mode can serve inline open-time prefetch.

## Failure Boundary

OwnerFs durability is the Home node local filesystem durability plus the Meta state that grants and tracks the workspace. Remote peer access is a transport path to Home, not an extra durable copy. If Home fails permanently, recovery depends on the deployment's local disk and Meta recovery policy.

An accepted ordinary write is not a durability acknowledgement. A successful size change through a remote open handle also requires the Home close-time barrier, even if that handle has issued no write. A fatal storage sync error remains attached to that open handle: later write, handle resize, flush and sync return the original error until release. Consuming a native writeback error does not turn a later close into a successful durability barrier. Reads and release remain possible; remote handles retain the same Home error. Retryable capacity and transport errors keep their existing retry rules.

After repairing the storage fault, open a new handle, explicitly rewrite any unconfirmed content and sync it. OwnerFs uses mutable local files: a failed sync does not promise rollback of accepted bytes. A successful earlier barrier defines the acknowledged watermark; overwritten bytes without a successful later barrier have no atomic rollback guarantee.

There is no automatic OwnerFs-to-DFS snapshot conversion in the base design.

## Optional Native Home Direction

The [native bind RFC](../rfcs/0001-ownerfs-native-bind-mount.md) targets a managed bind mount for colocated Home work. Functionality and performance have independent G2.12/G2.13 exits; a future explicit switch must default OFF. Current mainline contains private foundations, while production admission and a usable public ON setting remain unqualified. Ordinary FUSE delivery proceeds independently. ON requires final namespace/Root/epoch/Home checks, necessary file semantics, lifecycle/recovery and complete reference drain; a detached export alone does not revoke writable container clones. See [handoff](../handoff.md) for the current gaps.
