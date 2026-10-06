# Managed process termination

## Contract

`afs-processctl stop` reports the real child exit result for one managed incarnation. Native wait status, PID, executable, config, Linux boot ID, start ticks and launch directory are bound together. PID disappearance is not clean shutdown. Missing or mismatched results are unknown failures. `node`, `dfs` and `ownerfs` share one Node lifecycle lock; `all` controls Meta and Node. Failed or unknown termination blocks restart and uninstall.

A private supervisor in the existing controller owns `wait`. Unique launch records prevent stale receipt substitution. The ready/go handshake prevents an expired bootstrap from starting a product child; after go, incomplete publication retains launch intent for recovery. Readiness success rechecks original identity. Numeric identity validation and signalling are not pidfd protection; a killed controller can leave a mkdir lock, and unknown directories remain for diagnosis.

## Linux controller regressions

[Final report](qualified-controller/report.json): 16 labeled groups and 54 recorded CLI commands pass on ARM64 Ubuntu 24.04 build VM. The existing selftest uses native fixtures for exit0/1/124, SIGKILL137, unknown/mismatched/missing receipts, supervisor loss, repeat stop/status, blocked restart/uninstall, canonical alias concurrency, zero timeout, bootstrap SIGSTOP, delayed publication and immediate startup failure. [Raw output](qualified-controller/final.stdout) preserves command results. All five deployment scripts pass Linux bash syntax checks. Shellcheck was unavailable and was not claimed.

[Direct native comparison](qualified-controller/direct-exit-report.json) compares actual parent wait0/1/124 with controller0/1/124. The original false-clean FAIL remains unchanged in [prior evidence](../20261001-writeback/runtime-v48/processctl-exit-original-report.json). Initial r2 readiness-fixture failure and exact cleanup proof are retained; its shell interpreter was replaced by a native HTTP fixture without relaxing REST/mount readiness.

Controller SHA256: `b5dd0fa011475fb94658d3b1322ed8395c69aa1f85fbff5b5e1e4a307eec83a1`. Selftest SHA256: `71013d17e19dd95d6b454473cb8349cfa686d2eea090d1ffdf17cc22e17f2b99`.

## Actual product boundary

The isolated A runtime uses v49-qualified Rust binaries on guest ext4, memory Meta, R1 and gRPC/TLS, with OwnerFs and DFS mounts. Ports18080..18083 and `/mnt/lima-afsadata/afs-delivery/processctl-v50` are separate from preserved A/B v48 and v45. [Raw report](runtime-product/guest/report.json), [Root independent verification](runtime-product/root-verification.json) and [runtime description](runtime-product/README.md) bind hashes, configs, four receipt sets and first-read results.

Normal held-fd dirty write accepts14 bytes; Node stop returns0 with matching receipt/status; restart first-read returns `accepted-dirty`. Fault case first commits `seed` using fsync. The same Meta is paused and observed in stateT before the next write accepts4 bytes (~5.004s). Node stop returns124 (~20.124s), matching failed status and receipt, with `process.shutdown_forced` in the log. After resuming that exact Meta and starting a new Node incarnation, the first read is the previously acknowledged `seed`; the earlier normal file remains intact. Final Node/Meta stops return0, ports and mounts are clear. Root rechecks guest report, binary/config/controller hashes and final state independently.

The native 15-second watchdog begins when service shutdown starts. This run does not establish a 15-second signal-to-exit bound; measured controller stop is20.124s. Whole-shutdown budget and cooperative worker completion remain open. A forced exit is not a successful drain. Earlier prepause PASS, intermediate post-T PASS and the initial harness permission failure remain in runtime attempts; none is overwritten or counted as the final matrix.

Node SHA256: `dbbf2ccd5eb47f06178bfc3140599865c1b7ad5f28851e244e728fbcdfc136af`. Meta SHA256: `917432057950380da208b057a4d94156841e1eab03e533f56675433d5184fd9c`. Rust inputs match the [v49 source gate](../20261001-writeback/compile-inputs.json); no Rust code changes in this slice. v49 has only this isolated A lifecycle proof, not a new A/B full-suite qualification.

## Reproduction and limits

On the Linux build guest with native C compiler/Python dependencies, copy the five `scripts/deploy/` scripts together, run `bash -n` on them and execute `./selftest.sh`. [Product orchestrator](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-processctl/reproducers/processctl-runtime-probe.py) captures the research-workspace reproduction; it requires the documented Lima VM identities, existing qualified binaries/public TLS setup and isolated ext4 runtime. Preparation refuses an existing runtime instead of removing data. It is an environment-specific development probe, not the standalone release installer.

Full DEP/offline/idempotent installation, wider fault/backend/R=N/RDMA matrices, performance baselines, large-file and soak gates remain unqualified. Formal69-case release manifest remains NOT_RUN; ENV remains PREPARING. Overall goal is active. TLS private keys and binaries are absent from this evidence bundle. `docs/handoff.md` is unchanged.

[Artifact manifest](artifacts.json) records every file except this README and itself, including earlier attempts and the reproducer. Counts and SHA/bytes are integrity evidence, not a substitute for semantic assertions.
