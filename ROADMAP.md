# Roadmap

Engineering work advances by vertical capabilities that can be verified against the architecture contracts.

| Capability | Depends on | Acceptance result |
| --- | --- | --- |
| Local DFS file read/write | data model, local storage, Meta commit | FUSE create, write, sync, reopen and sparse read follow the contract |
| Cross-node mutable files | owner lease, ordering, cache invalidation | multi-writer and reader behavior has defined owner-failure results |
| Multi-replica durability | local finalize, placement, idempotent acknowledgements | configurable N/M policy and repair are verified under node loss |
| Fixed-version multi-source reads | stable versions, source catalog, grants | source changes preserve version and range identity |
| Consumer cache and seeds | full-chunk verification, soft directory, eviction | cache hits and seed withdrawal do not change durability promises |
| External spill and recall | external commit, reference tracking, eviction gates | capacity pressure does not delete the only valid source |
| DFS SDK data path | DFS read/write semantics, buffer lifecycle | batched I/O and low-copy paths keep the same file contract |
| OwnerFs completeness | root authority, Home, handle identity | local and remote workspace operations recover within the OwnerFs boundary |

Current progress is in [Implementation Status](docs/status.md).
