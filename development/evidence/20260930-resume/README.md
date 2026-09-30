# Linux continuation evidence

Local checkpoint date: 2026-10-01 (raw timestamps use UTC).

These are development results with exact candidate identities, not formal release approval. All 69 mandatory release cases remain NOT_RUN; ENV remains PREPARING.

## Actual upstream POSIX results

- `ownerfs-v25/full-pjdfstest/`: current strict driver, 236 discovered/executed files, 8819 TAP checks, zero unexpected failures/skips; 28 upstream TODO (9 not-ok, all TODO). Duration 155.647 s. The two original failing files separately passed 33 checks under `selected-pjdfstest/`.
- `dfs-v26/full-pjdfstest/`: current strict driver, 236 discovered/executed files, 8819 checks, zero unexpected failures/skips; 28 upstream TODO. Duration 1218.784 s. These counts belong to the identified v26 memory runtime.
- Original v11 and v25 product/probe failures remain in the research workspace. No exclusion was added to obtain these results.

## Lease and distributed lock results

- `renewal-before-fix.log` preserves three failing Meta expiry/identity regressions; `renewal-after-fix.log` preserves their passing repair. The full Meta contract has 38 passing tests.
- `ownerfs-v25/cross-locks.json` and `dfs-v26/cross-locks*.json` use different Linux VM kernels and actual product FUSE mounts. Each reports 7/7 passing steps. DFS's long wait is 35 seconds, longer than its 30-second lease period.
- `v27/` contains complete Linux gates, exact source hashes and actual A/B lock/fault proofs. OwnerFs and DFS pass 7/7 cross-mount steps; DFS also passes the 35-second wait.
- `v27/dfs-renewal-meta-stop/`: exact Meta PID/start ticks/boot identity/SHA bound before SIGSTOP; Linux state T observed, 18 seconds stopped, CONT attempted in finally. Holder/waiter stay alive; waiter acquires after unlock. This is a bounded pause, not restart or lease-expiry recovery.
- The v27 review found post-grant and delayed-success edge cases; the follow-up candidate and its separate results are recorded independently. v27 results do not qualify those later edits.
- `v28/` records strict Linux Clippy, 268 passing library tests (two explicit environment probes ignored), 56 interface tests, four shared-error tests, five privileged real FUSE tests, feature checks and the identified Node/Meta build. DFS cross-mount locks pass both six-second and 35-second waits; OwnerFs passes its seven steps. The exact-target 18-second Meta pause passes. A further review identified an in-flight renewal versus background expiry race; these v28 results do not qualify its subsequent fix.

## Test applicability and differential coverage

- `ltp-context/` preserves the repaired 657-command ext4 reference analysis. All 176 blocking events remain recorded. Of these, 104 filesystem-specific events are reference-only and cannot waive an AFS product failure; interpreting them as DFS correctly reports BLOCKED. This is a guardrail analysis, not an actual DFS LTP run.
- The hardened random driver binds the actual backend, mount and exact Node/Meta processes. A v27 DFS seed-one 500-operation smoke reached operation 187 and failed with EBUSY on pwrite where ext4 succeeded. Original trace and inconclusive Hypothesis replay remain in the research evidence; this original result is not a PASS. Later identified v30/v35 random slices are recorded separately in [authority evidence](../20261001-authority/README.md).

## Harness and failures

Portable Linux selftests passed 83 cases at v27 (79 prior plus 4 exact-target fault control tests). macOS runs only edit/copy/host orchestration; worker lock syscalls and tests are Linux.

The current LTP and lock/fault driver selftests additionally pass 34 cases together on Linux. Random-driver identity selftests pass 11 cases on Linux; those harness results do not qualify filesystem behavior.

The first privileged Cargo invocation attempted a separate root toolchain and was terminated. The already built root FUSE harness passed 5/5; then the shared-error catalog test found a missing TOML entry. Its original failing log and the corrected 4/4 error tests plus feature checks are retained. Staging/configuration failures are not product PASS.

Private keys, VM disks, compiled binaries and build caches are excluded. Source/binary hashes identify the observed machine; a new machine must rebuild and record its own identities. Mandatory POSIX/replica/transport/backend/resource/deploy/performance/long-run matrices still need completion.
