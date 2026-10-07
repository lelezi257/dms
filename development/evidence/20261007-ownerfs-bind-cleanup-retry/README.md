# OwnerFs workspace cleanup retry: bounded component regression

2026-10-07, main base `253057fdb94151283fcf3bae60c902c4098f67e1`. **Fact:** two new regressions reproduce two cleanup failures on the unchanged adapter, then pass with the narrow phase repair. G1 historical 8/8 stays closed; G2 counts and default OFF are unchanged. This is part of G2.12, not full ON qualification.

## Identity and evidence

| Phase | Compiler identity | Actual privileged test result |
| --- | --- | --- |
| Tests added, original cleanup | [157-file inputs](regression-inputs.json), map604b71a0bff8dd62a3f23732a333fef72a77db3761dabe5409816a3e7307639a | [exit101](regression/regression-command.json), [2 FAIL](regression/regression.log); test ELF SHA5373c38790e28c9c6dde642ce30812186270cac841a90ec304776d07bc149b8f |
| Fixed cleanup, same tests | [157-file inputs](fixed-inputs.json), mapa062bfe8737945e6258e67231fa13c87d04f744d24b838ad0a5fdb6a723d2872 | [exit0](fixed/fixed-physical-command.json), [2 PASS](fixed/fixed-physical.log); test ELF SHA75c8037b3590ef861f12514d2583030d6f28ddd2e3cdd9ce8a9dce63a71787c0 |

Existing ARM64 Linux afs-build, Rust1.95, ext4 release target and offline locked dependencies were reused. [Preflight](fixed/preflight.json) ran before compilation. [Seven selected gates](fixed/source-proof.json) all exit0: fmt, native-control (10 PASS/5 explicitly ignored), config-contracts (7 PASS), test ELF compilation, Owner-only and DFS-only compilation, strict Clippy. The two root cases ran explicitly after that test ELF was compiled; 590 discovered library tests does not mean 590 tests ran. Total actual selected test assertions: 19 test cases (10 ordinary + 7 config + 2 privileged).

**Binary scope:** no release service-build gate, package, deployment or official-runc E2E was selected. `source-proof.json.binaries` and `.ldd` files inventory pre-existing service executables only; they are not proof of executables rebuilt from this fixed compiler map. Only the above test ELF is qualified here. Previous [accepted Node run](../20261007-ownerfs-bind-node-accepted/README.md) and [6d performance measurements](../20261007-container-perf/README.md) retain their historical identities; no performance results are promoted or rerun.

## Behavior established

1. A real base bind mount held busy by a child cwd returns EBUSY after confirmed runtime deletion. The same Active/permit/export identity survives. After killing and waiting for that holder, retry detaches that export and reaches Idle without another runtime state/delete call.
2. A real secondary mount namespace clone is normally detached, then a controlled runtime delete fails. The retained claim remains Unknown. Retry skips the confirmed clone detach, completes delete/base detach and reaches Idle. Old code instead returned ESTALE.

The runtime fixture is a local controlled script, not runc; namespaces, mount IDs, bind mounts and EBUSY are real Linux operations. These tests directly call cleanup, not the socket. Public retry must use a new operation ID; the same ID intentionally replays its original result. Only confirmed successful stages advance. Failures retain the authority and mount; no forced/lazy unmount, missing-state success, reauthorization or vendor changes.

Full Cargo dependency-artifact JSON logs remain outside Git at the indexed paths/checksums in [build-log-index.json](build-log-index.json); portable summaries retain diagnostics and terminal build status. All original negative test output, commands, version maps, preflight and return codes are included. No maintained Python script, source snapshot, ELF, rootfs or private keys are copied.

## Remaining items

Node shutdown still has a single cleanup attempt and deadline-related ownership/drain gap; this repair does not solve it. Standalone host-visible bind enablement, native-FD revocation, production restart reconciliation and known mixed append/lock/watch semantics remain unfinished. Reuse unchanged historical timing data; next inspect the bounded Node shutdown ownership case before any claim of general safe drain. [Plan](../../ownerfs-bind-cleanup-retry.md). Main remains the only development/delivery entry; no feature PR/MR or human approval gate.
