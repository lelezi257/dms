# Positioning

AFS is a near-compute distributed file system for Agent, Sandbox and VM clusters. It exposes ordinary file paths and POSIX-style operations while using disks on compute nodes as the primary pool for hot data, durable replicas, verified cache and peer-to-peer reads.

AFS has two storage backends:

| Backend | Scope | Main benefit |
| --- | --- | --- |
| `DistributedFs` (`DFS`) | General shared distributed filesystem | Mutable files over immutable chunks, configurable replication, version-coherent reads, P2P, cache and optional spill |
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
- Successful `close` flushes prior writes and makes them visible to later opens; file sync does not imply parent directory sync.
- `fsync` does not create a business publish, alias, pin or snapshot.
- Verified cache does not count as a durable replica unless it is promoted and committed as one.
- External object storage is optional spill and cold capacity, not the mandatory source of truth.

The [delivery acceptance scope](acceptance.md) keeps the complete case catalog. It is broader than the first colleague-trial package and includes later backend, RDMA and reliability lanes.

## Current Trial Order

The current delivery order is recorded in the [three-stage goal table](../development/trial-release-goals.md). The usable trial path is intentionally simple: `memory` for disposable demos, `local-file` Meta for restart recovery, OwnerFs and DFS through FUSE, and package selfchecks that a colleague can run directly. etcd is a later resource/reliability topic, allowed to use a larger memory budget while investigated, and Redis is last.

G2 raises confidence and performance in small independent steps. Standard suites (`pjdfstest`, a fixed LTP subset and short fixed-seed FSx) are the fallback baseline; custom cross-node and restart cases supplement them. OwnerFs has priority over DFS. Within DFS, one-writer/many-readers is the first performance scenario.

The delivery acceptance page keeps the broader case catalog, including etcd/Redis persistence, FUSE and RDMA. DFS SDK, verified data cache and spill remain part of the broader architecture and are outside the current trial checkpoint unless a specific G2/G3 item names them.
