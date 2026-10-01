# Replica-repair unknown acknowledgement proof

## Result and scope

The [real gRPC test](../../../src/node/rpc/data.rs) verifies that replica repair retains exact claim/report identities after Meta has committed but its reply is withheld. This is a Linux development stage gate, not formal REL-04 or complete delivery acceptance.

The fixture uses a real memory-backed Meta service, source worker, Meta-authorized gRPC replica receiver and guest filesystem Chunk stores. A test-only service layer waits for the real handler response before delaying its first claim and report reply. Client deadline is 200 ms; the injected delay is 2 seconds. Plain loopback transport is deliberate test scope. Production protocols, modules, dependencies and runtime behavior are unchanged.

## End-to-end assertions

1. First worker tick times out after the claim has committed. The task is Running with `repair-claim:source-session:1`; the persisted outcome exists and no replica payload has moved.
2. The second tick replays that claim, transfers exactly 64 KiB once, then times out after the report has committed. Meta has two Ready checksum-matching copies, Satisfied placement and a Completed task. The store revision advances only once between the two snapshots: exact claim replay did not mutate it.
3. The third tick replays `repair-report:source-session:2`. Exact task, placement, copies, outcome identities and revisions stay unchanged. Receiver payload count remains 64 KiB, and physical target bytes still match.

[Static review](static-review.md) is separate from execution evidence. [Final compile inputs](linux/compile-inputs.json) bind 143 files to the current source; [Linux identity](linux/linux-identity.json) records the actual toolchain/build guest.

## Validation levels

| Level | Result | Evidence |
| --- | --- | --- |
| Local regression | PASS | [Exact failure replay](linux/lint-fix/wire.log), [data RPC](linux/local-second-r1/data.log) 19 PASS + 1 existing ignore, [replication](linux/local-second-r1/replication.log) 25 PASS, [Meta contracts](linux/local-second-r1/meta-contract.log) 39 PASS; 83 distinct selected tests, not 84. [Final strict lint](linux/lint-fix/clippy.log) and [format](linux/lint-fix/fmt.exit) PASS. |
| Stage source gate | PASS | [Library](linux/full/lib.log) 396 PASS + 2 existing environment ignores; [contracts](linux/full/contracts.log) 58 PASS; [shared errors](linux/full/error.log) 4 PASS; [local API](linux/full/local-api.log) 9 PASS; [actual privileged FUSE](linux/full/fuse.log) 5 PASS. [Strict workspace lint](linux/full/clippy.log), [format](linux/full/fmt.exit), feature checks and [binary build](linux/full/build.log) PASS. |
| Inventory script regression | PASS | [Nine Linux tests and available guest observations](environment/README.md); these do not qualify ENV-01. |
| Formal acceptance | NOT_RUN | 69 case entries unchanged; environment lock remains PREPARING. |

Every selected invocation records a command, raw log and exit code in [the Linux evidence](linux/). The [runner](linux/run-linux.sh) separates local, lint-fix and final batch gates. The full source gate ran once after fixture corrections; it was not repeated for each edit. Previous production/RXE runtime evidence retains its original v62-r3 identity and is not relabeled as v63 runtime evidence. Changed test code invalidated the old source gate for this candidate, so the final gate is fresh. No new deployment-script behavior requires restarting old fixtures.

## Original failures and remaining limits

The [original failed fixture](linux/local-original-r1/wire.log) expected a `DfsNamespace` wrapper for a direct `DfsReplicationClaim` outcome. Its [input hashes](linux/inputs-original-r1.json) are preserved. The subsequent [strict Clippy failure](linux/local-second-r1/clippy.log) identified two collapsible test match branches; [its source identity](linux/inputs-second-r1.json) remains. Successful reruns do not turn those original failures into PASS.

This does not prove unknown `CommitFileVersion` inode blocking, TLS/RXE acknowledgement loss, cross-VM link failure, durable Meta restart, full POSIX or long resource lifetimes. Existing no-feature/OwnerFs-only compile warnings about an unused private quarantine helper remain recorded; strict all-feature lint has no warnings. Full 8 GiB, performance matrices and stability tests stay at their scheduled acceptance phases. Acceptance thresholds, AGENTS and `docs/handoff.md` remain unchanged.
