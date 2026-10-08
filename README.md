# AFS

**当前交付：bind ON有限场景试用已发布。** [b80 ON试用](https://github.com/lelezi257/dms/releases/tag/afs-bind-b80dab6)保持原身份，G2.12有限出口关闭。[READ一次改善](development/evidence/20261008-owner-remote-read-frames/README.md)吞吐+14.21%、独立pread p95-4.29%已保留。[最新WRITE摸底](development/evidence/20261008-owner-remote-write-rpc-cost/README.md)数据已保存，目录预算漏算导致本轮FAIL_BUDGET；内容及三正常退出分别核对，不计性能通过。[纯Owner节点入站帧候选](development/evidence/20261008-owner-remote-write-server-frames/README.md)源码检查通过并外置保留，生产已恢复；正式写对照待夹具空间选择；Moose双目标仍待验，不重包。DFS写按用户决定暂缓；G1历史8/8关闭，原G2 12/0/15不变，普通本地及复杂可靠性后置。

AFS is a near-compute distributed file system for Agent, Sandbox and VM clusters. It exposes file interfaces with [same-mount visibility and close-to-open consistency](docs/architecture/write-semantics.md), while using disks on compute nodes as the primary pool for hot data, durable replicas, verified cache and peer-to-peer reads.

AFS has two backends:

- `DistributedFs`: the general DFS path for shared files, immutable chunks, file versions, replicas, P2P reads, cache, repair and optional spill.
- `OwnerFs`: the small-cluster workspace path for 1 to 4 nodes. A workspace has a Home node; local work uses normal local files, and remote work goes back to the Home through P2P.

![AFS architecture](docs/images/overview.svg)

`afs-meta` owns namespace, inode records, file versions, placement, leases and lifecycle metadata. `afs-node` runs next to workloads, owns FUSE mounts, orders writes, stores chunks on local disks, serves peer reads and manages cache. File bytes do not pass through Meta.

## Read This First

1. [Documentation Home](docs/README.md)
2. [Three-stage trial and acceptance goals](development/trial-release-goals.md)
3. [Current source checkpoint](development/current-checkpoint.md)
4. [Positioning](docs/positioning.md)
5. [Architecture](docs/architecture.md)
6. [Data Model](docs/architecture/data-model.md)
7. [Write Semantics](docs/architecture/write-semantics.md)
8. [Implementation Status](docs/status.md)

[Delivery Acceptance](docs/acceptance.md) defines the detailed case catalog. The current execution order is the three-stage table: first a usable OwnerFs/DFS trial with `memory` demo and `local-file` Meta restart recovery, then small-to-large core performance, then long/complex reliability and remaining persistence backends.

Current priority is:

- G1: historical `g1.5` colleague-trial scope is complete for its stated range.
- G2: required usage is OwnerFs workspace bind **ON** with correct remote FUSE access and an explicit ON trial. Prioritize finite bind functions/delivery, remote performance, DFS one-writer/many-readers, then ordinary local FUSE. Reuse scoped standard and eight bind core >=0.90 ext4 passes; tools/documents are supporting work. Ordinary Owner read/write still requires >=1.2x MooseFS throughput and <=0.8x independent operation latency; DFS keeps three-synchronous-copy matched 3FS parity.
- G3: long soak, broad fault matrices, etcd memory/resource work and Redis persistence are deferred.

OwnerFs workspace bind mount has separate function and performance gates. Ordinary defaults remain OFF; the published b80 ON trial closes the selected finite scenario, with its explicit support limits and historical eight-case performance scope.

## Development

[Delivery handoff](docs/handoff.md) records the execution checkpoint, remaining gates and portable continuation inputs.

The authoritative build and runtime environment is Linux. The Rust toolchain is pinned by [rust-toolchain.toml](rust-toolchain.toml).

Use the guides for local commands:

- [Quickstart](docs/guides/quickstart.md)
- [Configuration](docs/guides/configuration.md)
- [Operations](docs/guides/operations.md)
- [Validation](docs/guides/validation.md)

Historical default-OFF Linux ARM64 trial: [historical7e6 package instructions](docs/guides/trial-7e6.md) and [7e6 reproducible archive / installed recovery](development/evidence/20261007-current-trial-7e6/README.md). The [fixed prerelease](https://github.com/lelezi257/dms/releases/tag/afs-trial-7e6e00a) is published; its four remote asset digests are confirmed by the publication receipt there. The [6d checklist](docs/guides/trial-6d.md) remains historical. Complete G2 performance/ON gates remain open.

Current default-OFF Linux ARM64 trial: [f03 instructions](docs/guides/trial.md), [fixed prerelease and downloads](https://github.com/lelezi257/dms/releases/tag/afs-trial-f03dc2b), [fresh reproducible package/install/recovery and publication evidence](development/evidence/20261008-current-trial-f03/README.md). Both workspace switches remain OFF. Current f03 results keep their scope; historical standards retain their versions, and full G2 performance remains open.
