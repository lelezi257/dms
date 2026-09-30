# AFS Source Repository Instructions

Read these files before changing this repository:

1. `README.md`
2. `PRINCIPLES.md`
3. `docs/README.md`
4. `docs/positioning.md`
5. `docs/architecture.md`
6. `docs/status.md`

## Documentation Rules

- Product documentation describes the accepted target design.
- Implementation capability status belongs in `docs/status.md`; `docs/handoff.md` carries the portable execution checkpoint.
- Refresh `docs/handoff.md` only when the user explicitly requests it; ordinary development and validation do not trigger a handoff refresh.
- Mechanism pages live under `docs/architecture/` and should explain the user-visible scenario first, then the contract.
- Historical process notes are not product documentation; use Git history for old stage records.
- SVG diagrams live under `docs/images/` and should use readable text labels.

## Code Rules

- Do not change runtime behavior in a documentation-only task.
- OwnerFs and DFS are separate mounts and separate backend state machines.
- SDK references are DFS-only unless a later accepted design says otherwise.
- Run validation from Linux for filesystem or Rust build claims.

## Delivery implementation

- [docs/acceptance.md](docs/acceptance.md) is the first-stage release gate; do not lower it.
- Before implementation read [implementation rules](development/implementation.md) and [validation strategy](development/validation.md). These hold detailed rules; do not duplicate them here.
- Read [delivery handoff](docs/handoff.md) when continuing implementation on this or another machine.
- Read [delivery task map](development/plan.md) for dependencies. In the research workspace use `../execution/README.md` for the current checkpoint and change log; standalone clones keep execution artifacts under ignored `.local/delivery/`. Architecture/interface changes require a final-review record.
- Use the locked Linux environment; macOS is only for editing and VM orchestration. Keep current module and RPC boundaries.
