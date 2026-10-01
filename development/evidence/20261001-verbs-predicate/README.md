# Cross-VM verbs environment consumer

This batch validates acceptance tools on Ubuntu ARM64 Linux. It does not
qualify a product release, a formal RDMA case or the complete environment.

## Results and scope

| Check | Result | Evidence |
| --- | --- | --- |
| Dedicated predicate regressions | 17 methods PASS | [Final local log](linux/final-fixed-local.log), [tested inputs](linux/final-fixed-inputs.sha256) |
| Related environment/network/runner regressions | 60 methods PASS | [Final batch log](linux/final-fixed-related.log) |
| Necessary Python compilation | PASS | [Compile exit](linux/final-fixed-compile.exit), [invocations](commands.jsonl) |
| Actual retained preparation bundle | 37 PASS / 9 BLOCKED / 0 FAIL; overall BLOCKED | [Preparation report](linux/final-fixed-preparation.json) |
| Real evaluator/runner consumer | PASS: nine qualification errors, no driver execution | [Consumer result](linux/consumer.json), [Linux harness](linux/consumer_check.py) |
| Unchanged original Rust gate | Reused under its original identity | [143 input hashes](rust-reuse.json) |
| Static independent review | No identified blocking findings after repair | [Review](review.md) |

These are local and affected-batch validation results. The original v64 Rust
gate remains applicable because all 143 compile inputs are unchanged; it is
not a newly executed Rust gate. No formal case is promoted. The release lock
remains PREPARING and the 69 registered formal cases remain NOT_RUN.

## What the consumer proves

The optional `verbs` block in the [bundle](bundle/bundle.json) references the
retained v69 preparation files. The evaluator requires supported frozen
collector/checker/observer sources, artifact and raw-log digests, all twelve
nonself directions plus the 65,535-byte boundary exchange, exact roles and
invocations, endpoint/process/boot identity, route/GID/MTU, full READ/WRITE
contents, MR descriptor receipts, the real absent-listener rejection and
post-run resource cleanup. It recomputes paired semantics with the supported
checker; a generic PASS receipt is insufficient.

Matching command intervals must cover the endpoint captures. An explicit
50 ms host/guest record-matching tolerance accommodates the observed original
millisecond-scale skew. It does not qualify the separate clock-accuracy gate.
Missing evidence remains BLOCKED; contradictory or altered evidence is FAIL.

The [248 retained artifact references](bundle/bundle.json) keep their original
metadata/network/verbs capture identities. This batch reruns their consumer,
not the transfer matrix. The original collector differs from the final
checker. The protected-before observer was a separate bootstrap without a
historical command transcript; no attestation is fabricated for it. Stock
server peer authorization is limited to endpoint route/GID and descriptor
evidence. Installed provider hashes do not prove loaded provider mappings.

## Preserved failures

- [Original consumer](linux/consumer-original.log) rejected the expected verbs
  promotion because the old evaluator deferred this prerequisite. This first
  bootstrap predates the batch command journal; its timestamp is not inferred.
- [Old evaluator against dedicated tests](linux/dedicated-original.log):
  12 methods, 32 failing subcases.
- [Initial candidate](linux/candidate-local.log): 12 methods, seven failing
  subcases. Raw newline-bearing boot/machine IDs and a legitimate empty C
  process list were not normalized/accepted correctly. The
  [candidate preparation failure](linux/candidate-preparation.json) is retained.
- [Normalized local run](linux/normalized-local.log): 12 methods PASS.
- [Resource/negative-identity review regressions](linux/review-original.log):
  16 methods, seven failing subcases before hardening. The
  [hardened run](linux/hardened-local.log) passes 16 methods.
- [Time-binding regressions](linux/time-original.log): 17 methods, three
  failing subcases. Exact client/server/negative commands from a different
  month incorrectly passed before interval binding.
- [First interval-binding run](linux/final-local.log): 17 methods, one failing
  missing-command subcase. Repair preserves missing evidence as BLOCKED.
- [Final run](linux/final-fixed-local.log): all 17 methods PASS. Earlier
  candidate snapshots and input manifests retain their original identities.

The final [acceptance input snapshot](acceptance-final/environment.py) and
[dedicated tests](acceptance-final/test_environment_verbs.py) match the Linux
hash manifest. The [host orchestration helper](validate.mjs) invokes all Python
inside the Linux guest. The [Linux identity](linux/identity.txt), raw logs,
exit statuses and [command journal](commands.jsonl) are retained. The runner's
READY/FROZEN identities in the consumer are deliberately synthetic harness
inputs: its purpose is to prove that the real environment validator refuses
dispatch, not to qualify those inputs for acceptance.

## Remaining prerequisites

The nine blocked checks are initial host reserve, durable-backend restart,
complete ext4 reference-suite accounting, actual MooseFS mount I/O, actual 3FS
mount I/O, complete frozen inputs, formal run contracts, clock accuracy and
cgroup/mount/cache/thin-allocation semantics. They are neither inferred from
preparation flags nor waived. Product RDMA lifecycle/security/fallback,
backend parity, full POSIX, 8 GiB, performance and stability retain their
declared acceptance stages.

No product Rust, RPC, persistence format, deployment script or dependency
changes are included. [AGENTS and handoff hashes](protected-files.json) are
unchanged. The [selected validation scope](selection.md) explains reuse and
the separate local, stage and formal result levels.
