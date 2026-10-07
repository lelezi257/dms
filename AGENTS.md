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

- [Three-stage acceptance checklist](development/trial-release-goals.md) owns current scope, priority and independent exits; [docs/acceptance.md](docs/acceptance.md) preserves the expanded case catalogue. Do not weaken a selected case's correctness or durability requirements.
- Before implementation read [implementation rules](development/implementation.md) and [validation strategy](development/validation.md). These hold detailed rules; do not duplicate them here.
- Read [delivery handoff](docs/handoff.md) when continuing implementation on this or another machine.
- Read [delivery task map](development/plan.md) for dependencies and [current checkpoint](development/current-checkpoint.md) for portable source/validation identities. Research-workspace artifacts remain outside the product repository; standalone clones keep new local execution artifacts under ignored `.local/delivery/`. Overall project review occurs after the agreed overall goal; normal affected build/test/commit checks continue without per-feature human approval gates.
- Use the locked Linux environment; macOS is only for editing and VM orchestration. Keep current module and RPC boundaries.
- Main is the sole daily development and delivery entry. Commit independent completed items directly on main and normally push origin/main under the user's standing authorization. Do not create a feature PR/MR or add an individual review/approval gate unless the user separately requests it. Preserve historical branches/worktrees for provenance; do not rewrite history or force push. [Convergence inventory](development/evidence/20261007-main-convergence/README.md) records included and retained work.
