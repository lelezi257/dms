# Node controller startup rejection and orderly cleanup

2026-10-07. **PASS for this bounded startup-error path.** [Plan](../../ownerfs-bind-node-startup-slice.md), [commands and oracle correction](commands-receipt.json), [frozen compiler map](compiler-inputs.json), [manifest](SHA256SUMS). G1 historical8/8, G2 task counts and default OFF are unchanged.

| Identity | Recorded value |
| --- | --- |
| Source base | df56e4850a62ccca712aa495d8bba51d148f9900 |
| Compiler inputs | 157 files; map SHA256 196a4177141fb837d3e5b155438e894ab9d632bfceb97a9ea3ab20bf2ce57c6d |
| Only compiler-input change | src/node.rs; SHA256 dde9b50e50892309acb2f6036b967d184163a7b02ec43d1dc434aa5fa01fbac8 |
| New release Meta ELF | f4d423fcced38cd0a6624b1104ff05c136974150d21590522a90a40e637aca0c |
| New release Node ELF | de1d16895c062651aa9e9de375b9ac5bb5da5b52672a7d70093bebdd090bc9ee |
| Actual test ELF | b5705a479a5b01aaad879ab5f24c1bfb78b287f154d50ba3f7132a9dec72de37 |

[Linux source proof](build/source-proof.json): fmt, affected release test compilation, Owner-only and DFS-only feature checks, strict workspace/all-target/all-feature Clippy and release binaries all exit0; inputs unchanged. [Five Node shutdown tests](build/node-unit.log) pass, 583 filtered out, [actual exit0](build/node-unit.exit). Two new regressions use real Services and an already-started sibling: cancellation, shutdown hook and cleanup finish before returning; original errno13 survives a later cleanup error; an actual blocking-task panic retains its JoinError. The panic text is expected fault injection.

The production fix removes the early .await?? return after FUSE and services have started. It registers initialization failure in the existing service owner, retains the original error separately, drains services and reaches Node's existing explicit cleanup. Failed startup does not announce readiness. Configuration, core bind behavior and vendored code are unchanged.

[R2 result](runtime-r2/result.json), [all28 checks](runtime-r2/checks.json), [raw Node log](runtime-r2/node.stderr), [commands](runtime-r2/commands.json): fresh isolated /opt/afs-bind-node-startup-20261007-r2, memory Meta, generated TLS, valid adapter configuration with an intentionally absent trusted runtime path. Configuration inspection succeeds; actual Node [wait returns1](runtime-r2/node-wait.json), no node.ready; the same log stream observes services.stopped before the unique node.shutdown_failed carrying ENOENT/os error2. LocalAPI socket and controller artifacts are absent. Meta [actual wait returns0](runtime-r2/meta-wait.json). Complete mountinfo and existing AFS process identities match [before](runtime-r2/before.json) and [after](runtime-r2/after.json); all157 compiler inputs remain fixed, budget/free-floor checks pass.

The [first run FAIL](runtime-r1/result.json), [failure](runtime-r1/failure.json), raw logs, tool SHA and actual waits remain immutable. The driver initially expected an Os/NotFound error. Existing NativeWorkspace::start already wraps worker errors using io::Error::other(error.to_string()), and the CLI prints the precise Custom/Other ENOENT string. Only that exact oracle was corrected; source, ELF and environment did not change. R2 was necessary because the tool changed. Existing standards, core FUSE and performance tests were not repeated. No dependency installation, environment repair or forced cleanup occurred.

This proves rejected controller startup and the Node cleanup return for this case. It does not qualify accepted workspace startup, full ON/G2.12, standalone host visibility, general reference drain, mixed-path semantics, POSIX completeness, performance or durable Meta recovery. These ELFs are not a newly installed trial package. Historical [3cc orderly recovery](../20261007-native-orderly-recovery-runtime/README.md), [165d ownership repair](../20261007-ownerfs-bind-remediation/README.md) and [df56 real FUSE core](../20261007-ownerfs-bind-core-fuse/README.md) retain exact identities and scopes.

Next follow G2.12 accepted Node/workspace lifecycle before G2.13 performance. Main convergence does not transfer every historical runtime PASS to this compiler map.

[Publication checks](publication-checks.json):157 current inputs match, local evidence links resolve, protected handoff unchanged, third-party diff empty. Linux SHA256 manifest verification passes.
