# DFS cancellation and remote handle lifecycle

Date: 2026-10-01. Identified v40 Linux ARM64 A/B development candidate, memory Meta, R=1, gRPC and independent FUSE mounts. No full release or performance gate is qualified here.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Restore old cancellation behavior | Both targeted regressions FAIL | [Before](linux-before-cancel.log) |
| DFS targeted regressions | 91 PASS | [After](linux-targeted-dfs.log) |
| Formatting, strict all-target/all-feature Clippy, feature checks and binary build | PASS | [Qualified Linux gate](linux-integration-qualified.log) |
| Library / interface / shared error / privileged actual FUSE | 300 / 57 / 4 / 5 PASS; two explicit library environment ignores | [Qualified Linux gate](linux-integration-qualified.log) |
| Compile inputs | All 143 host/Linux inputs match | [Source manifest](linux-source-hashes-qualified.json) |
| A/B OwnerFs/DFS consistency | Ten PASS | [Report](consistency/report.json) |
| A/B DFS locks with concurrent namespace operations | Seven PASS; blocking wake after 35.4008 seconds; interruption within unchanged 55-second bound | [Report](dfs-cross-under-load/report.json) |
| Namespace load | Three rounds / 3842 operations; stable process/mount identities; fixture cleaned | [Result](namespace-load/result.json), [Events](namespace-load/events.jsonl) |
| Complete DFS lock run overlaps namespace load | PASS; 75 observed process identities match exact PIDs, SHA and stable start times | [Verification](load-overlap-verification.json) |
| A/B OwnerFs locks | Seven PASS | [Report](owner-cross/report.json) |
| A/B DFS locks during full remote pjdfstest | Seven PASS; wake after35.349 seconds; unchanged55-second interruption bound | [Report](dfs-cross-during-full-remote/report.json) |
| Full upstream workload overlaps the lock run | PASS; suite process identities stable, TAP output advances,75 product identity records match | [Verification](dfs-cross-during-full-remote/overlap-verification.json) |

## Mechanism

An interrupted blocking lock wait returns its interruption result without a redundant synchronous cancel/ACK or a post-cancellation lease renewal. Its exact waiter route remains available for background acknowledgement. The owner-side regression forces a blocked renewal, so restoring the old post-mutation call fails deterministically. Both original failing assertions are preserved.

Remote file handles bind inode, caller node/session and lease epoch. Reads, writes, resize and sync validate that scope. Release validates the exact local handle scope while allowing cleanup after its original write lease advances; unrelated stale errors are not treated as successful release.

Failed releases retain the original opaque handle and all authority fields. Admission reserves cleanup capacity before owner open. A fair pending queue retries with a 250ms maintenance budget, a 2s release-drain budget and at most 100ms per release RPC. Cancelled waiter acknowledgements and peer session reaping also use bounded passes and short RPCs. Production gRPC timeouts cover connection and request work. Unknown Meta session state retains resources.

## Identities and limits

Node SHA256: `19c0fda458e12708c38b577aa82c0b9b307facfff2c74f0ff17571d5af10fee3`.
Meta SHA256: `2f6a4f6e219468f056e9749030683c20d5432beb94bf7009e7e28f47681238e5`.
A Node PID703297 / Meta PID703267; B Node PID10887. The consistency probe captures exact identities, separate boot IDs and actual mounts. The namespace probe checks A identities before and after the load. These are observed run values, not installation defaults.

The preserved [v37 failure](../20261001-owner-rename/v37/dfs-cross/report.json) occurred with a complete namespace suite running concurrently on A. The first v40 rerun uses a synthetic create/rename/stat/unlink load. The additional rerun uses the same seven lock cases and time bounds while the full remote upstream suite runs on B; the controller and prove processes remain stable and TAP bytes advance from4795 to17554 during the lock run. The complete upstream result is still pending. This verifies the focused lock repair under both observed loads; sustained resource qualification remains open. Earlier v37 full POSIX results do not qualify v40 or additional backends.

Known remaining lifecycle work includes owner-open ACK loss before the caller learns its handle, retirement of pending release identities after an owner process changes, retry RPC amplification and a total Node shutdown/writeback budget. Bounded cleanup RPCs do not bound the existing dirty-inode drain. No REL-10/11 release PASS is claimed. Full RDMA, durable-backend, repair, storage, installation, fair performance and long-run gates remain open; formal69 cases remain NOT_RUN and ENV PREPARING.

All compilation, tests and product I/O run on Linux. macOS performs editing, copying, hashing and host orchestration. No executable or private key is published. The qualified build includes the error catalog and examples and matches all143 tracked compile inputs; the earlier incomplete capture is not qualification evidence. The handoff document is unchanged.
