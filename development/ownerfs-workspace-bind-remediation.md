# OwnerFs workspace bind mount: narrow naming and ownership repair

2026-10-07. Independent high-priority item after the ongoing orderly-recovery slice is saved. G1 remains8/8; no whole-repository refactor, vendor edits, new production enablement or historical verdict changes.

## Facts and behavior boundary before changes

Current Driver::start obtains an authorized physical Home directory FD and clones it onto OwnerFs FUSE root/workspace. The controller thread first enters a private mount namespace; ordinary host/Node callers still see FUSE. runc then binds that private first-level view to /workspace. Source/target satisfy the underlying-directory and first-level path shape in that private view, but this does not provide ordinary-caller visibility or a standalone runc-free Node lifecycle. Existing production admission is disabled and the experimental adapter defaults OFF. Renaming cannot close these gaps or the existing append/classic-lock/watch/reconciliation failures.

## Scope and sequence

1. Preserve the current candidate/package/runtime evidence and any first collector failures before product edits. Record the exact old physical source, first-level target, namespace isolation, second container target and Node call points.
2. Move the existing descriptor-confined mounting implementation and physical tests from src/node/native_workspace/mount.rs to the single src/node/vfs/ownerfs/bind_mount.rs; no bind_mount directory. Name its owned claim WorkspaceBindMount. Its API owns mounting, namespace/directory/mount identity checks and normal detach. It must not mention runc, native_workspace or container_mount in core ownership/names, nor hardcode a container /workspace target.
3. Parameterize secondary clone inspection/detach by a validated directory component. The runc adapter selects workspace; all runc start/exec/stop/rootfs/socket/process logic stays in its current separate adapter file. Node keeps existing lifecycle wiring. Do not widen mount namespace visibility or authority as part of this extraction.
4. Keep existing experimental_native_workspace / native_workspace TOML and CLI unchanged as compatibility-preserving runc-adapter options, explicitly document their scope. No silent renamed keys or claim of a new standalone bind switch. The bind capability remains default OFF through existing admission.
5. Reuse existing trust, no-follow, namespace/unique-ID/flags, foreign-mount/identity-drift/proc-hidden clone tests; preserve normal umount flags0, errno propagation and retained claim on failure (no existing EBUSY test is claimed). Add a meaningful non-container component regression for the generic clone API; reject any newly introduced unsafe source/target ambiguity without weakening Home/root/epoch/access-generation, freshness, permissions, error or drain requirements.
6. ARM64 Linux only: run affected ordinary tests, actual privileged bind tests and config contracts; update source-slice-linux.py with a precise new bind physical filter, retaining the three native rootfs privileged checks so a module move cannot silently skip the four migrated physical tests; fmt/strict affected Clippy/build as warranted. Preserve any failures. Exact file/input identity changes get a new manifest, never inherit old-package runtime PASS. Do not repeat standard suites, ordinary performance or the already-finished runtime slice without a changed behavior reason.
7. Update current design/status/guides/examples and evidence index, keep old paths/results traceable through a move mapping and immutable packets. Independent review of the final diff precedes Lore commit and normal GitHub push. Return to the pre-existing E2E order; feature/full ON acceptance remains separate.

## Acceptance for this repair

One core file at required OwnerFs path; no dependency on runc or adapter-module visibility; arbitrary validated clone target component (no fixed container path); unchanged adapter lifecycle/config parsing/default OFF and actual low-level identity/normal-unmount behavior proved by targeted Linux checks. Report physical Home source and private first-level FUSE target truthfully. Ordinary-caller/standalone bind lifecycle and full ON safety are remaining functional items, not completed by this repair.

**Independent plan review:** native_lock_review APPROVE with the two clarifications above. Generic target-component validation must precede setns or mount effects. Review was read-only; implementation and evidence review remain separate.

**Final narrow exit (2026-10-07):** [Current fixed-source evidence](evidence/20261007-ownerfs-bind-remediation/README.md): 25 selected distinct Linux tests plus fmt/strict Clippy/release build PASS; independent final static review APPROVE. Core path/names repaired, compatibility/default OFF unchanged. Ordinary-host/standalone bind/full ON still pending. No historical acceptance replaced.
