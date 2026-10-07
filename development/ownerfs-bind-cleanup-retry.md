# Retry confirmed workspace cleanup stages

2026-10-07, main253057f. This is a bounded G2.12 correctness repair before further workspace qualification; G1 stays8/8. Existing6d paired performance data remains historical, without repetition or a current performance PASS.

## Defect and change

Current adapter cleanup normally detaches a captured final clone, deletes its container, then detaches its base OwnerFs export. A failed delete repeats the already completed clone detach; a successful delete followed by EBUSY repeats state lookup for a removed container. Both prevent a new public Stop from completing while the owned Active survives.

Record only confirmed monotonic final-clone detach and container-delete stages in Active. New Stop continues remaining work on that same permit/export, without preparing a new mount, accepting missing runtime state as success, or touching foreign claims. Keep native control IDs/replay/defaultOFF and core module ownership unchanged; no vendor changes or dependencies.

## Coverage before repair

Existing physical core tests cover identity drift and busy detach but do not cover adapter cleanup ordering. Add Linux privileged regression cases using actual mounts and an external cwd holder plus a deliberately controlled runtime fixture: delete succeeds then base umount EBUSY; clone detach succeeds then delete fails. Assert same original claim after failure, no repeated completed stage, and eventual Idle after the cause is released. These are adapter/component tests, not actual runc/Node E2E. First run them against the unchanged cleanup to retain expected failure, then apply repair and rerun selected cases. Also run existing ordinary adapter checks, config/defaultOFF and affected fmt/Clippy/build checks as warranted, in existing ARM64 Linux only.

## Boundaries

This does not implement shutdown retry ownership across the Services/Node deadline, standalone host-visible enablement, generalized grant revocation/native-FD fencing, or full ON/performance qualification. Preserve those unfinished items explicitly; do not couple this repair to a whole runtime refactor. Main is the only entry, direct Lore commit and normal push; no feature PR/MR or approval gate. Save actual tool/environment blockers and stop the affected lane.

## Closure

The unchanged cleanup produced the two expected actual Linux FAIL results; the phase repair produced two PASS on the same privileged cases. Seven selected source gates passed; 10 ordinary controller/Node tests and 7 config cases also passed. Original negative output and exact compiler/test identities are retained in [the packet](evidence/20261007-ownerfs-bind-cleanup-retry/README.md). No new service ELF/package/deployment is claimed; shutdown ownership and general drain remain unfinished.
