# Remote DFS full-suite timeout and accounting

Date: 2026-10-01. Linux ARM64 A/B development candidate v40, memory Meta, DFS R=1, gRPC. This run is BLOCKED by the original 1800-second bound. It does not qualify full STD-01 or a performance gate.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Full remote pjdfstest | BLOCKED at 1800.016 seconds; prove terminated, no final suite summary | [Host proof](host/artifacts/std-01-pjdfstest/proof.json), [Worker command](worker/artifacts/std-01-pjdfstest/command.json) |
| Observed checks | 7961; zero unexpected assertion failures, zero observed skips, 27 upstream TODO; incomplete coverage | [Original TAP](worker/artifacts/std-01-pjdfstest/pjdfstest.stdout.tap) |
| Observed file completion | 170 of 236 selected files; 66 unobserved; last observed file `rmdir/02.t` | [Corrected accounting](corrected-accounting.json) |
| Captured raw artifact integrity | All seven byte counts/hashes match; worker proof equals the host embedded record | [Verification](manifest-verification.json) |
| Corrected driver and identity Linux selftests | 51 PASS | [Raw selftest](linux-selftest.log) |
| Reparse retained v37 complete DFS output | 236 observed completed files / 8819 checks; complete accounting remains true | [Reparse](v37-complete-reparse.json) |

## Accounting contract

The original timeout proof incorrectly reported all236 selected files as executed. That original proof and its hashed raw output are retained unchanged. Corrected accounting is a separate Linux-derived artifact, bound to the original TAP and original accounting SHA.

The driver now separates selection, observed starts, observed completion, incomplete observed output and unobserved files. A missing file header does not prove the file never started: prove can buffer an entire file before output reaches the captured stream. Successful complete accounting requires all selected files to be observed completed, matching prove file/check totals and matching TAP planned/observed checks. Timeout remains BLOCKED; no bound or exclusion changes.

The [seven-step cross-node lock result](../20261001-dfs-lifecycle/dfs-cross-during-full-remote/report.json) passes while this workload is active. Its complete overlap and stable identity proof remain valid, but a focused lock result does not make this incomplete namespace suite pass.

## Identity and remaining work

A Meta PID703267, B Node PID10887. Node SHA256 `19c0fda458e12708c38b577aa82c0b9b307facfff2c74f0ff17571d5af10fee3`; Meta SHA256 `2f6a4f6e219468f056e9749030683c20d5432beb94bf7009e7e28f47681238e5`. Strict remote pre/post checks retain process start times, endpoint, configuration, TLS digests, independent VM and actual FUSE identities in the host proof. These are observed values, not installation defaults.

The unfinished fixture is retained on B. Completion latency requires investigation and an identified full rerun within the same bound. Formal69 release cases remain NOT_RUN and ENV PREPARING. Memory-backed development results do not qualify persistent backend or final release matrices. All execution and parser selftests run on Linux; macOS only edits, copies, hashes and orchestrates. The handoff document is unchanged.
