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
- Implementation progress belongs only in `docs/status.md`.
- Mechanism pages live under `docs/architecture/` and should explain the user-visible scenario first, then the contract.
- Historical process notes are not product documentation; use Git history for old stage records.
- SVG diagrams live under `docs/images/` and should use readable text labels.

## Code Rules

- Do not change runtime behavior in a documentation-only task.
- OwnerFs and DFS are separate mounts and separate backend state machines.
- SDK references are DFS-only unless a later accepted design says otherwise.
- Run validation from Linux for filesystem or Rust build claims.
