# AFS Documentation

## Start Here

- [Three-stage trial and acceptance goals](../development/trial-release-goals.md)
- [Current source checkpoint](../development/current-checkpoint.md)
- [Positioning](positioning.md)
- [Architecture](architecture.md)
- [Delivery Acceptance](acceptance.md)
- [Implementation Status](status.md)
- [Delivery Handoff](handoff.md)

The three-stage goal table is the current execution authority. `acceptance.md`
keeps the full case catalog, while the goal table defines which subset is G1,
which core standard/performance cases are G2, and which long or complex work is
G3. G1 is complete for the historical `g1.5` trial scope; current source
publication is a separate checkpoint and must not be described as having passed
all G2 gates until those cases are rerun.

## Mechanisms

- [Data Model](architecture/data-model.md)
- [Write Semantics](architecture/write-semantics.md)
- [Replication](architecture/replication.md)
- [Local Storage and COW](architecture/local-storage.md)
- [Read, Cache and Spill](architecture/read-cache-spill.md)
- [OwnerFs](architecture/ownerfs.md)
- [Meta and Transactions](architecture/meta.md)
- [Module Map](architecture/module-map.md)

## Guides

- [Quickstart](guides/quickstart.md)
- [Configuration](guides/configuration.md)
- [Operations](guides/operations.md)
- [Trial Package](guides/trial.md)
- [Validation](guides/validation.md)

## Proposals

- [RFC Index](rfcs/README.md)
- [RFC Template](rfcs/0000-template.md)
