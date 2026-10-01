# Independent cross-VM verbs preparation

## Result

**Local regression PASS; scoped preparation batch PASS.** Fourteen Linux
probe methods pass, including five new failure-replay methods.
The [final raw-evidence audit](audit.json) records **127 PASS, zero FAIL**.
The twelve non-self ctl/A/B/C directions at 256 bytes and three iterations
each complete; A→B also completes three 65,535-byte iterations.
An A→B invocation after the listener exits fails explicitly (shell exit 255,
stock return -1), with no payload success.

The 39 iterations provide 39 successful server READ and 39 WRITE completions,
matching complete echoed payload digests. Each endpoint has six SEND/RECV
completions per exchange: 156 of each across 26 endpoints. The 78 client MR
advertisements match server receipts. SEND here carries 16-byte control
descriptors/go-ahead, not bulk user data. Descriptor digests reconstruct
network-order fields; they are not packet-capture checksums.

The actual tool is rdma-core 50.0-2ubuntu0.2 `/usr/bin/rping`, SHA256
`d4d82fdd9b78cfb9404bbb4c2a8c07dde9d450d7fd4d7041d5d0ffcf32e3e89f`.
The [upstream v50.0 source](runtime/ctl/upstream/rping.c) explains the
descriptor→READ→go-ahead→descriptor→WRITE→go-ahead exchange, full client
memcmp, printed ASCII payload and trailing NUL. Its provenance is
[linux-rdma/rdma-core](https://github.com/linux-rdma/rdma-core/blob/v50.0/librdmacm/examples/rping.c);
it is reference source, not a claim that the Ubuntu package is byte-identical.
Payload digests include the trailing NUL. MTU records distinguish Ethernet
1500 from verbs active MTU 1024.

## Scope and identities

[Selection](selection.md), [matrix](matrix.json), [command transcript](commands.jsonl)
and [run index](runs.jsonl) bind exact commands, UUIDs and endpoints.
The frozen [wire collector](inputs/env_verbs.py) and
[executed orchestrator](inputs/orchestrate-executed.mjs) remain under their
original hashes. The [final checker](inputs/env_verbs-checker.py) corrects
parser/evaluator behavior only; transport capture was not rerun or relabeled
as executing the new source. Binary/guest/kernel/GID/route/provider/process
observations and raw logs are under [runtime](runtime/).

Installed provider/library fingerprints are recorded; they do not attest
loaded-library mappings. The supported stock tool uses RDMA CM/verbs without
a TCP payload fallback. This is a standalone exchange, not an AFS file API,
bulk SEND content test or hardware-performance result.

All four protected AFS process identities, configurations and mount lists
match before/after. No probe-owned QP or CM ID remains in the final snapshots.
The observer was copied and invoked separately by root before the executed
matrix; [the initial unprivileged failure](observation-original.md) is retained.
The audit checks its frozen source against each guest copy, guest boot/machine
identity and before→endpoint→after timestamp ordering. It does not invent a
historical pre-execution hash or command timestamp for that separate bootstrap.
The updated [orchestrator](orchestrate.mjs) includes observer copying and
before-capture for future use; that updated version was not run in this batch.

## Regression history

- [Initial nine methods](runtime/ctl/initial-tests.log) pass but omit the
  concrete live parser cases.
- [Initial live audit](audit-original.json): 102 PASS, 13 FAIL. The route tool
  prints `DEST from SOURCE dev eth0`; the first checker accepts only src/prefsrc.
  Eleven clients also omit the asynchronous CM disconnect log before exiting.
- [Review regression](linux/review-original-tests.log): three failures and
  one error across thirteen methods retain those false negatives, zero-count
  and comment-based false PASS, and an oversized descriptor exception.
- [Intermediate thirteen methods](linux/final-tests.log) pass. A final review
  found missing UUIDs could still pass; [the targeted replay](linux/uuid-original-tests.log)
  preserves both failing subcases in one method.
- [Final fourteen methods](linux/final2-tests.log) and
  [Python compilation](linux/final2-compile.rc) pass. Exact route tokens,
  bounded/typed request identity, anchored lifecycle records and structured
  malformed-input failures close those gaps. A nonempty valid UUID is required.
  [Final static review](review.md) has no remaining identified blocker.
- Stock client cleanup calls disconnect, joins the CQ thread and frees its
  QP/buffers; the asynchronous CM print is not guaranteed before exit.
  Client proof requires successful complete transfer, actual cleanup and
  resource disappearance. Server disconnect evidence remains required.
  Formal resource-lifetime requirements are unchanged.

## Acceptance boundary

The existing ENV evaluator still reports 36 PASS/10 BLOCKED and defers its
cross-VM verbs predicate. The real lock remains PREPARING and all 69 formal
cases remain NOT_RUN. No ENV driver promotion, product RDMA-required/fallback
PASS, performance, POSIX, 8 GiB or stability result is claimed.

Rust inputs are unchanged; [reuse](rust-reuse.json) binds the original 143-input
v64 gate. [Protected files](protected-files.json) retain AGENTS and handoff.
Only probe/audit/Python checks and real scoped operations ran; no repeated full
Rust gate. [Artifact hashes](artifacts-manifest.json) preserve raw results.
The overall delivery goal remains active.
