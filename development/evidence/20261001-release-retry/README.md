# Bounded remote release retries

Date: 2026-10-01. Linux ARM64 source candidate v41; the running A/B filesystem candidate remains v40. This source gate does not qualify a release or a performance result.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Restore the original retry loop | Two new regressions FAIL | [Original behavior, compressed raw log](linux-before-retry.log.gz) |
| Library regressions | 302 PASS; two explicit environmental ignores | [Qualified Linux gate](linux-qualified-gate.log) |
| Interface / shared error / privileged actual FUSE tests | 57 / 4 / 5 PASS | [Qualified Linux gate](linux-qualified-gate.log) |
| Formatting, strict all-target/all-feature Clippy, feature matrix and binary build | PASS | [Qualified Linux gate](linux-qualified-gate.log) |
| Compile inputs | All143 host/Linux inputs match | [Host manifest](host-source-hashes.json), [Linux manifest](linux-source-hashes.json) |

## Contract

A maintenance pass snapshots the pending release queue and attempts each selected identity once. It selects at most64 entries and retains the existing250ms aggregate and100ms per-RPC budgets. Successful replies remove only their exact entry. Failed replies preserve the entry and rotate it to the queue tail.

The first regression queues two immediately failing releases and requires exactly two calls. The second queues65 releases: the first pass attempts indices0 through63; the next pass starts with index64. Restoring the original loop fails both tests because it repeatedly selects the same failed entries until its time budget expires. Existing regressions verify successful later retries remove the exact queued identities.

The shutdown helper performs the same bounded pass with a2s budget. Remaining entries produce an error; this helper does not promise to empty an arbitrarily large queue. The overall dirty-inode shutdown drain remains separate, unfinished work.

## Identity and limits

Node SHA256: `71c0347eb2d7fe2f9f831079eadb2d3afda8dda86ea762b8339514a11b8d3b89`.
Meta SHA256: `a59ab784fbb41ac23bb818699549f020872c3ebe4b5564d74c6b2a2150814d8c`.

These binaries were built from the captured v41 source snapshot in the dedicated Linux build VM. They have not replaced the live v40 processes. The [v40 cross-node lock rerun](../20261001-dfs-lifecycle/dfs-cross-during-full-remote/report.json) separately passes seven steps during the remote full upstream suite, with [process and overlap verification](../20261001-dfs-lifecycle/dfs-cross-during-full-remote/overlap-verification.json). Those runtime results retain their own source and binary identities.

Owner-open ACK loss, retirement after owner-session replacement, overall shutdown budgets, sustained lifecycle faults and the remaining release matrix are unqualified. Formal69 cases remain NOT_RUN and the environment lock remains PREPARING.
