# Node-owned workspace shutdown: bounded source and kernel regression

2026-10-07, main base `59f87533f06c0c2a394bee148458915ce4ef9bb8`. **Fact:** Node retains the runc-adapter worker outside Services, waits for its closure before any FUSE join, and uses the existing process watchdog for failed closure. The same mount thread retries only raw EBUSY on its existing Active. Identity/unknown-runtime errors are reported and retained without automatic retries; listener fatal errors are reported before a possibly busy cleanup wait. Mount core ownership, default OFF, global15s budget, third-party source and dependencies are unchanged. This is a G2.12 subcase; G1 historical8/8 and G2 counts remain unchanged.

## Version and original failures

| Input | Exact proof |
| --- | --- |
| New busy-shutdown test, original product | [157-file map](regression-inputs.json), SHA9390747897d1d5741d0f304aefa1ab3d461eed82631a7c79a2c377ed87c3d58a; [actual exit101](regression/regression-command.json), [1 FAIL: EBUSY](regression/regression.log) |
| Initial repair test compile | [input map](fixed-inputs.json), [E0282 test watch payload type](compile-failure/native-control.log), [exit101](compile-failure/native-control.exit); explicit tuple type added, no environment repair |
| Intermediate r2 | [map](fixed-r2-inputs.json), [archive/proof index](intermediate-r2-index.json); selected source and four root checks passed but superseded by listener-notification fix; not current qualification |
| Final r3 | [157-file inputs](fixed-r3-inputs.json), map1fd613e55e28aac00c9514e74763501ec58aa83524956c2caf5ec1e675287e55; [source proof](fixed-r3/source-proof.json), inputs unchanged |

Existing ARM64 Linux afs-build/Rust1.95/ext4 target/offline locked dependencies were reused; [complete preflight](fixed-r3/preflight.json) precedes compilation. [Maintained tool identity](tool-identity.json) references Git and SHA, without copying its Python source. Large dependency-artifact Cargo JSON remains outside Git at [indexed checksummed raw logs](build-log-index.json); diagnostics/terminal summaries, all negative output, exact commands/maps/exits are portable.

## Current passing scope

Nine selected gates exit0: fmt, native-control, config-contracts, test compilation, privileged native adapter tests, Owner-only/DFS-only checks, strict Clippy and release service build. Actual distinct executed test cases: **26 = 11 ordinary + 7 config + 8 privileged**. [Ordinary output](fixed-r3/native-control.log) discovers19 in the selected filter, executes11 and explicitly ignores8; [root output](fixed-r3/physical-native.log) explicitly executes those8, with586 library tests filtered. No full library, POSIX suite or performance comparison was run.

Four newly added cases establish:

- The actual Services10s timeout/abort completes while a controlled worker remains Node-owned; explicit join still waits for that same worker. This is lifecycle wiring with a controlled worker, not actual FUSE teardown or whole-process E2E.
- A real bind mount held by child cwd remains busy during shutdown, then normally detaches after kill+wait; the container delete is not repeated and control socket/lock close only after success.
- A controlled terminal ESTALE is observed once; a child records the retained physical mount/control identities before the existing short test watchdog exits124. That exit is failure, not normal closure. This is a kernel/component test with controlled runtime, not production recovery or native-FD revocation proof.
- A selected fatal listener-error branch first wakes the actual Services monitor/callback, arming the existing short watchdog while real cwd/directory-FD references still hold the bind mount; normal drain waits until those references are released. The original EMFILE errno propagates. The injected error exercises the same production branch; it is not actual kernel FD exhaustion.

The previous two cleanup-phase regressions and three rootfs tests also pass. The child helper is a plain function behind the parent's environment branch, not a discovered no-op/standalone ignored test. [Exact root argv](fixed-r3/physical-native.command.json). [Postcheck](postcheck.json): no relevant test processes or caller-namespace test mounts; test ELF SHA94cffe459d2208244c444bea93015c1b0e02f06d11bf3a1037f08cf56d285979.

## New build versus runtime candidate

Final release Meta SHA78e7ad556b8767af4bcc34c81fb94075011152254e26ed24b26a8f528b97d3a8, Node SHAd264a470372593ab00bd6c691f1e389492decf8e4c750a6b29b608719407d58a, helper SHAc9c9fd1a3568c872fb975add7595df43f46016c674647ef48ac2103db979ddb7. These were actually built, not the pre-existing inventories in earlier no-build runs. No new package or installation occurred here. [Readonly runtime candidate identity](runtime-candidate-identity.json) confirms installed1451f60 ELFs retain f4d/de1 identities and no Meta/Node was running on afs-g2-micro at observation.

[Accepted1451f60 Node/runc run](../20261007-ownerfs-bind-node-accepted/README.md), [59f cleanup-phase check](../20261007-ownerfs-bind-cleanup-retry/README.md), [6d timing data](../20261007-container-perf/README.md) and historical standards preserve their own versions; no runtime/performance results are promoted to this changed source.

## Remaining next case

Package these identified current ELFs and run one current official-runc/Node active-workspace shutdown case on the existing isolated Linux VM, without first clearing the workspace via public Stop. Include actual process/mount closure receipts before upgrading runtime qualification. Full host-visible bind switch, general native-FD fencing/revocation, restart reconciliation and mixed append/locks/watch remain open. The binary entry has its existing15s watchdog; embedded node::run has no automatic process deadline and cannot be claimed bounded by this test. No standalone switch, vendor adaptation, global runtime rewrite, ordinary timing retune or G1 reopen is included. [Execution plan](../../ownerfs-bind-shutdown-drain.md). Main only, direct Lore commits/normal push.
