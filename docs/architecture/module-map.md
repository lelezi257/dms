# Module Map

## Meta Components

| Area | Files | Responsibility |
| --- | --- | --- |
| Process assembly | `src/meta.rs`, `src/bin/afs-meta.rs` | configuration, transport and service startup |
| DFS metadata | `src/meta/dfs.rs` | namespace, inodes, leases, file versions, layout, copy and placement transactions |
| OwnerFs metadata | `src/meta/owner_roots.rs` | workspace root ownership and grants |
| RPC adapter | `src/meta/rpc.rs` | wire/domain conversion, validation and error mapping |
| Store | `src/meta/store.rs`, `src/meta/store/` | committed state, condition checks, batching and persistence backends |

## Node Components

| Area | Files | Responsibility |
| --- | --- | --- |
| Process assembly | `src/node.rs`, `src/bin/afs-node.rs` | mount lifecycle, local services and peer services |
| FUSE module | `src/node/fuse.rs`, `src/node/fuse/state.rs` | shared FUSE request handling and per-session tables |
| Backend trait | `src/node/vfs.rs`, `src/node/vfs/types.rs` | common filesystem operation shape |
| DFS backend | `src/node/vfs/dfs.rs` | write ownership, dirty view, commit and committed reads |
| OwnerFs backend | `src/node/vfs/ownerfs.rs`, `src/node/vfs/ownerfs/` | Home local files and peer workspace operations |
| OwnerFs workspace bind mount | `src/node/vfs/ownerfs/bind_mount.rs` | descriptor-confined mount, identity checks and normal unmount; no runc dependency |
| Runc workspace adapter | `src/node/native_workspace.rs` | experimental container lifecycle and secondary clone selection; Node wires startup/shutdown |
| Local chunks | `src/node/chunk.rs` | immutable chunk staging, finalization, catalog and reads |
| Replication | `src/node/replication.rs` | R=1 and R=N execution shape and receipts |
| Fixed-version reads | `src/node/dfs_read.rs` | extent-to-chunk read planning and source attempts |
| Peer RPC | `src/node/rpc/data.rs`, `src/node/rpc/peer.rs`, `src/node/rpc/control.rs` | Node-to-Node data and control |
| Meta RPC client | `src/node/rpc/meta.rs` | Node-to-Meta calls |

## Boundary Rules

- Meta never transfers steady-state file content.
- OwnerFs and DFS are separate mounts, not virtual roots in one mount.
- DFS chunk replication lives below the file layout layer.
- Read batching, attempts and source selection are Node runtime concerns.
- Cache and spill must use copy role/state rules before serving or evicting data.
- NodeControl owns Owner RDMA negotiation and transport close. OwnerFiles owns file commands and payload descriptors. Owner sessions are isolated from DFS and diagnostic sessions; transport identity never replaces file authorization.

## DFS Transport Selection

`grpc` carries file bytes through the existing bounded streams. `rdma` requires
an available configured device and keeps every transport failure explicit.
`auto` prepares RDMA when the local feature and device are available, and prefers
it for chunk replication and fixed-version reads.

An authenticated peer can report that RDMA is unsupported during negotiation.
Auto may then use gRPC before sending a data command, within the same operation
deadline and with the same authority. Capacity, authorization, protocol,
integrity and uncertain completion errors do not permit write replay.

A read batch may contain several RDMA windows. Once any window's data command
has been sent, the batch cannot restart through gRPC. The read engine handles
failed source attempts using its fixed-version and scratch-buffer contract.
Actual completed byte counters identify the data path; a preferred mode alone
does not prove RDMA traffic.
