# OwnerFs workspace bind mount: Node startup failure cleanup

2026-10-07. G2.12 independent admission/lifecycle fix, source base df56e4850a62ccca712aa495d8bba51d148f9900. Preserve G1 historical8/8 and default OFF. Do not change core mount behavior, configuration, third-party code or existing acceptance conclusions.

## Missing behavior and coverage

Node starts FUSE and Services before awaiting NativeWorkspace::start. The current `.await??` returns immediately on controller initialization failure or blocking-task panic, bypassing Services::run_with_shutdown and Node's explicit FUSE join/backend cleanup. Existing Node tests check heartbeat retry and first-error retention, not this startup failure path.

## Plan

1. Keep initialization awaited and preserve the original error. Register either the successfully initialized controller supervisor or a failed startup service in the existing Services owner. Preserve the existing behavior of not emitting node.ready for a failed controller. Keep the original startup error separately as Node's first error, since later service drain errors can otherwise replace it. Route that error through the same service stop/drain and subsequent explicit Node cleanup path; do not change ordinary OFF startup or successful controller behavior.
2. Add meaningful targeted asynchronous regressions: an initialization io::Error retains its errno even when a sibling cleanup also fails, while already-started sibling services observe cancellation and finish; an actual blocking-task panic/JoinError likewise triggers stop/drain. Verify the shutdown hook and that cleanup completed before return. Test the exact production registration function and readiness result, not a duplicate of the implementation.
3. ARM64 Linux only: freeze source input map, one dependency/capacity/idle preflight through the maintained source driver; fmt, affected release test compilation, strict workspace Clippy and owner-only/dfs-only feature checks. Execute exactly Node shutdown tests from the actual hashed test artifact. No unchanged standard/performance/FUSE-core rerun.
4. Preserve failures and raw commands, update the current ledger and GitHub evidence, Lore commit/normal push. Follow the latest user authorization: main is the daily development/delivery entry, no per-feature PR/MR or human approval gate; overall project review occurs after the overall goal. A real environment blocker stops this lane and prompts assistance instead of repairing the environment.

## Scope and exit

This closes the startup-error control-flow defect. Unit evidence proves cancellation/error propagation through real Services, not whole-process FUSE teardown or generic native drain. A candidate package/runtime startup-failure check remains necessary before claiming actual Node process cleanup; any new ELF needs its own identity. Existing controller-cleanup failure/timeout followed by FUSE Drop and general host visibility/reference-drain boundaries remain open, as do full ON/POSIX/performance acceptance.

## Candidate process negative case

Use the maintained node-startup-rejection-linux.py driver with newly built, hashed Meta/Node release ELFs and the same frozen source map. One fresh root under /opt on guest ext4, 256MiB owned budget/1GiB free floor, ports24800/24801/24900/24901, memory Meta and generated TLS. Valid experimental adapter config points to an intentionally absent absolute runtime file beneath the trusted owned root; no runc or bind is invoked. Require print-config success, Node actual wait1 with original ENOENT, no node.ready, services.stopped then final node.shutdown_failed carrying original os error2, terminal exact existing Custom/Other ENOENT string, LocalAPI socket/controller artifacts absent; Meta actual wait0; original complete mountinfo and existing AFS process identities unchanged. Save commands/config paths/raw logs/hashes; retain fixture/TLS outside Git. This proves this rejected startup and clean return, not accepted bind startup, normal runtime workspace lifecycle, durable Meta recovery or generic drain. Root path changed before execution from preparatory /var/tmp idea because the existing trusted_path rejects writable ancestors; no environment change or failed product run.

## Result

[Bounded evidence](evidence/20261007-ownerfs-bind-node-startup/README.md): six selected Linux gates pass; five Node tests pass; real R2 startup rejection passes28 checks, actual Node wait1/Meta wait0. R1 tool-oracle FAIL is preserved; no product/environment change. This closes this startup-error control flow, not accepted ON or general drain.
