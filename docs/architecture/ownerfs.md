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

## OwnerFs workspace bind mount

For a colocated Home workspace, the accepted design mounts the workspace's real Home directory onto its corresponding first-level directory under the OwnerFs FUSE root. For example, the physical root/epoch directory backs /ownerfs/agent1; after the covering bind is active, fresh path accesses in that namespace use the underlying filesystem. Binding the FUSE directory itself elsewhere still uses FUSE and does not satisfy this design. Existing FUSE handles keep their original references until released.

The core implementation belongs to the single src/node/vfs/ownerfs/bind_mount.rs file. WorkspaceBindMount takes an already-authorized Home source descriptor and an OwnerFs root descriptor plus one validated workspace component. It records directory, namespace and unique mount identities, applies nosuid/nodev, and detaches normally only while the exact owned claim remains visible. Home/root/epoch/authorization, shared cache policy and operation admission remain OwnerFs/Node responsibilities; a low-level mount is not a file-access grant.

Containers are an adapter scenario. src/node/native_workspace.rs selects runc, starts/executes/stops its workload and drains its owned references before calling the OwnerFs core; Node wires this adapter before FUSE shutdown. Generic secondary-clone inspection/detach takes a validated component; the adapter chooses workspace for its /workspace target. The core does not depend on runc or a fixed container target.

Current behavior is narrower than the accepted generic capability: the adapter mounts the physical directory at OwnerFs root/workspace only inside its controller's private mount namespace, then binds that view into the container. Ordinary host/Node callers keep the original FUSE view. Standalone admission, ordinary-caller visibility, multiple-workspace lifecycle and abnormal recovery remain functional work. This extraction does not enable them or bypass authorization at uncontrolled file-operation boundaries.

Function and performance remain independent G2.12/G2.13 gates and default OFF. Existing experimental_native_workspace TOML / --experimental-native-workspace CLI and the native_workspace section retain their exact parsing and semantics as legacy-named runc-adapter settings; no renamed alias or silently ignored key is introduced. They are not a standalone bind feature switch. Full ON requires grant/freshness/close-to-open/permissions/error semantics and reference drain/recovery; append offsets, classic locks and watch propagation remain known gaps. [RFC and historical mechanism results](../rfcs/0001-ownerfs-native-bind-mount.md), [implementation status](../status.md) and [narrow repair plan](../../development/ownerfs-workspace-bind-remediation.md) keep design, implementation and acceptance separate.

The [one real-FUSE core integration test](../../development/evidence/20261007-ownerfs-bind-core-fuse/README.md) now passes on Linux ARM64: physical Home backing storage covers the matching FUSE first-level directory, controlled local fresh-open reads match in both directions, cwd-held detach returns EBUSY and retains identity, and normal teardown restores FUSE. This private-namespace/fake-Meta component test does not close standalone Node lifecycle, host visibility, general reference drain or full ON/performance acceptance.
