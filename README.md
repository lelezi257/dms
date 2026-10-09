# AFS

**当前交付：[a103阶段性ON试用包已发布](https://github.com/lelezi257/dms/releases/tag/afs-bind-a103a2f)。** Linux ARM64，可编译、可独立安装、可运行；真实Home bind ON及Owner64KiB/DFS64MiB正常全停local-file恢复限定通过。普通配置默认OFF，按[明确ON指南](docs/guides/trial.md)启用。[固定身份、证据与支持限制](development/evidence/20261009-workspace-bind-a103-trial/README.md)。新Meta远端组合、完整远端POSIX、性能目标、复杂可靠性及官方fuser迁移仍未完成；G1历史8/8和原G2 12/0/15不变。[d47历史ON包](https://github.com/lelezi257/dms/releases/tag/afs-bind-d47eec2)及原判据/结论保留。

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

OwnerFs workspace bind mount has separate function and performance gates. Ordinary defaults remain OFF; the current a103 ON trial provides the selected finite scenario, with explicit support limits and separately scoped historical eight-case performance evidence.

## Development

[Delivery handoff](docs/handoff.md) records the execution checkpoint, remaining gates and portable continuation inputs.

The authoritative build and runtime environment is Linux. The Rust toolchain is pinned by [rust-toolchain.toml](rust-toolchain.toml).

Use the guides for local commands:

- [Quickstart](docs/guides/quickstart.md)
- [Configuration](docs/guides/configuration.md)
- [Operations](docs/guides/operations.md)
- [Validation](docs/guides/validation.md)

Historical default-OFF Linux ARM64 trial: [historical7e6 package instructions](docs/guides/trial-7e6.md) and [7e6 reproducible archive / installed recovery](development/evidence/20261007-current-trial-7e6/README.md). The [fixed prerelease](https://github.com/lelezi257/dms/releases/tag/afs-trial-7e6e00a) is published; its four remote asset digests are confirmed by the publication receipt there. The [6d checklist](docs/guides/trial-6d.md) remains historical. Complete G2 performance/ON gates remain open.

Historical f03 default-OFF trial: [fixed prerelease and its packaged guide](https://github.com/lelezi257/dms/releases/tag/afs-trial-f03dc2b), [original package/install/recovery evidence](development/evidence/20261008-current-trial-f03/README.md). Its results retain their original version and OFF scope. The current a103 trial and explicit ON guide are linked above.

[Finite integration handoff](development/integration-closeout.md) records the frozen build/package identities, source migration scope and unfinished capabilities; GitCode integration has not yet been performed.
