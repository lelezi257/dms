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
