# Authority and remote-view validation

Date: 2026-10-01. Scope: identified Linux ARM64 development candidates with memory-backed Meta. These results do not qualify the full backend, performance, deployment or release matrix.

## Qualified v36 short results

| Check | Result | Evidence |
| --- | --- | --- |
| Strict Clippy, formatting, feature checks and Node/Meta build | PASS | [Linux gate](v36/linux-integration-retry2.log) |
| Library regressions | 289 PASS, two explicit environmental probes ignored | [Linux gate](v36/linux-integration-retry2.log) |
| Interface contracts, shared errors, privileged actual FUSE | 57 / 4 / 5 PASS | [Linux gate](v36/linux-integration-retry2.log) |
| Compile input identity | All 143 host/guest inputs match | [Source manifest](v36/linux-source-hashes.json) |
| OwnerFs/DFS consistency | 8/8 PASS, 28 recorded commands | [Actual A/B report](p2-consistency-v36/report.json) |
| OwnerFs cross-mount locks | 7/7 PASS, 35-second wait | [Report](p2-locks-v36/owner-cross/report.json) |
| DFS cross-mount locks | 7/7 PASS, 35-second wait | [Report](p2-locks-v36/dfs-cross/report.json) |
| Exact Meta pause target | 2/2 PASS, 18 seconds stopped, four signal/control records | [Report](p2-locks-v36/dfs-renewal-meta-stop/report.json) |
| DFS/ext4 random differential | Seed 1, 500/500 PASS, 180-second functional bound | [Report](p2-random-v36/README.md) |

Node SHA256: `646a67667d434bbbe771e183558a0ae894976eaeb6e0c5b0e2d28864b61cbd99`.
Meta SHA256: `06fb8d24b9ad02aebbeb1e3aeb5f7daa7dc516deef27a68dda2bfd2d5168d150`.

The reports bind different Linux VM boot identities, exact process PIDs/start times/executable hashes and actual AFS FUSE mounts. Runtime paths are observations on the recorded machine, not portable acceptance identities. File I/O and Cargo verification run on Linux; macOS performs editing, copies, hashing and VM orchestration only.

Consistency assertions are one-shot. They cover same-mount dirty data and length, close-to-open, remote-owner writes, handleless resize with an existing write-only handle, and a former local owner's remote dirty view after handover. Local write-only reads remain rejected; a read-capable companion serves authorized same-mount readers. Protected dirty/pending state cannot be silently rebound or bypassed.

The pause test checks live lock renewal across a bounded interruption. It is not a Meta restart, expired-lease recovery or permanent source-loss test. The 35-second lock waits are separate checks against the observed 30-second request timeout. Random replay is a small functional slice, not the full ten-seed/10,000-operation gate or a performance result.

## Preserved negative and earlier observations

- Meta shorter-open expiry and clock-skew authorization failing regressions remain beside their qualified repaired gates.
- v30 runtime reports 6/7 original consistency checks; remote truncate was rejected. Its seed-one 60-second replay was INCONCLUSIVE at 327/500; a separate 180-second run passed 500/500. The longer bound changes this functional diagnostic, not any release performance threshold.
- v31 has an original Clippy error and then a protected getattr-mode regression failure. It has no qualified runtime candidate.
- v32 source passed 282 library tests; the original seven consistency scenarios passed. Both new provider test failures remain preserved under `p2-consistency-provider-v32/`.
- v33 source passed 283 library tests; review found a provider restoration race before deployment.
- v34 source passed 284 library tests; actual expanded consistency passed 6/8. Original local-clock rejection and stale zero-length EOF failures remain recorded.
- v35 source passed 285 library tests. Original consistency passed 7/8: fresh readonly access via a write-only provider returned EBADF. Expanded handover coverage passed 6/8 and exposed retained local state masking remote dirty bytes. Its locks, random replay and bounded Meta pause have separate positive reports; these do not turn its consistency failures into PASS.
- v36 original library run failed because the routing test lacked a readable committed base at its new owner. The next attempt had a fixture type error. Both logs and source manifests remain. The final gate retains the full behavior assertions and passes all 289 tests.
- v36 random preflight originally reported BLOCKED because its fixture base did not exist. The preserved separate rerun has a valid unique base, exact identity and 500/500 PASS.

Earlier [complete pjdfstest and harness evidence](../20260930-resume/README.md) binds its own candidates. No exclusion was added to obtain these short results. No binaries, build cache, VM disks or private keys are published here.

## Remaining gates

The formal 69-case manifest remains NOT_RUN and the environment lock PREPARING. Required durable-backend parity, full applicable POSIX/concurrency/security, R=N repair and faults, RDMA/fallback matrices, bounded resource/capacity/crash behavior, install/restart matrices, fair MooseFS/3FS comparisons, full FSx/random/8-GiB and eight-hour soak remain open. Failed remote release can still require owner-side retry/session cleanup; the local route-cleanup regressions do not qualify that full reliability boundary.
