# Contributing

Start from [Architecture](docs/architecture.md), then use [Implementation Status](docs/status.md) to choose an implementation gap.

## Design Changes

- File semantics, persistent format, version identity, replication, lifecycle, wire protocol and module-boundary changes need a design proposal when the behavior is not already accepted.
- Accepted design is folded into the relevant page under `docs/architecture/`.
- Use [RFCs](docs/rfcs/README.md) only for unresolved proposals.

## Documentation Changes

- Keep `docs/README.md` as navigation.
- Put product positioning in `docs/positioning.md`.
- Put system overview in `docs/architecture.md`.
- Put mechanism contracts under `docs/architecture/`.
- Put implementation progress only in `docs/status.md`.

## Validation

Use [Validation](docs/guides/validation.md) for check selection. Pull requests should state the behavior changed, affected contract, verification evidence and untested boundary.
