# AFS

AFS is a near-compute distributed file system for Agent, Sandbox and VM clusters. It exposes file interfaces with [same-mount visibility and close-to-open consistency](docs/architecture/write-semantics.md), while using disks on compute nodes as the primary pool for hot data, durable replicas, verified cache and peer-to-peer reads.

AFS has two backends:

- `DistributedFs`: the general DFS path for shared files, immutable chunks, file versions, replicas, P2P reads, cache, repair and optional spill.
- `OwnerFs`: the small-cluster workspace path for 1 to 4 nodes. A workspace has a Home node; local work uses normal local files, and remote work goes back to the Home through P2P.

![AFS architecture](docs/images/overview.svg)

`afs-meta` owns namespace, inode records, file versions, placement, leases and lifecycle metadata. `afs-node` runs next to workloads, owns FUSE mounts, orders writes, stores chunks on local disks, serves peer reads and manages cache. File bytes do not pass through Meta.

## Read This First

1. [Documentation Home](docs/README.md)
2. [Positioning](docs/positioning.md)
3. [Architecture](docs/architecture.md)
4. [Data Model](docs/architecture/data-model.md)
5. [Write Semantics](docs/architecture/write-semantics.md)
6. [Implementation Status](docs/status.md)

[Delivery Acceptance](docs/acceptance.md) defines the release scope, fixed Linux VM environment, functional suites, performance targets, reliability, RDMA and installation gates.

## Development

The authoritative build and runtime environment is Linux. The Rust toolchain is pinned by [rust-toolchain.toml](rust-toolchain.toml).

Use the guides for local commands:

- [Quickstart](docs/guides/quickstart.md)
- [Configuration](docs/guides/configuration.md)
- [Operations](docs/guides/operations.md)
- [Validation](docs/guides/validation.md)
