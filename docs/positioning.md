# Positioning

AFS is a near-compute distributed file system for Agent, Sandbox and VM clusters. It exposes ordinary file paths and POSIX-style operations while using disks on compute nodes as the primary pool for hot data, durable replicas, verified cache and peer-to-peer reads.

AFS has two storage backends:

| Backend | Scope | Main benefit |
| --- | --- | --- |
| `DistributedFs` (`DFS`) | General shared distributed filesystem | Mutable files over immutable chunks, configurable replication, fixed-version reads, P2P, cache and optional spill |
| `OwnerFs` | Small Agent workspaces, normally 1 to 4 nodes | Home-node local files with peer forwarding when compute moves away from Home |

DFS is the general path. OwnerFs exists because a small workspace often gets better latency and simpler failure boundaries when the active data remains on its Home node.

## Workload Fit

AFS is designed for:

- Agent workspaces that are edited locally but sometimes accessed remotely.
- Shared mutable files that need normal write, sync and reopen behavior.
- Images, snapshots, checkpoints and datasets that become fixed file versions and can be read from many sources.
- MicroVM disk files where the base image is fixed and the writable layer produces new chunks.

AFS does not require a separate Blob API. Images and snapshots are stable file versions in DFS. Their chunks already have immutable identities that support cache, P2P and spill.

## Boundaries

- AFS does not claim every POSIX workload is fastest on AFS.
- `close` does not commit data.
- `fsync` does not create a business publish, alias, pin or snapshot.
- Verified cache does not count as a durable replica unless it is promoted and committed as one.
- External object storage is optional spill and cold capacity, not the mandatory source of truth.
