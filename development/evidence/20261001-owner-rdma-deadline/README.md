# Owner posted-RDMA deadline

The Linux ARM64 RXE fixture uses the public synchronous OwnerPeerClient and the real mTLS OwnerFs handler. GDB pauses only the native worker after a successful 4096-byte READ post and before CQ consumption. The actual RPC deadline expires; aborting a spawn_blocking handle is not used as cancellation evidence.

## Result and scope

The later [coherent batch source gate](../20261001-round1-mainline/README.md)
passes with these exact final test inputs: 414 library/65 contract/4 shared-error/
9 local API/5 privileged FUSE, fmt, strict Clippy, five feature configurations
and builds. Earlier input-audit records retain their pre-gate snapshot and scope;
the actual ignored posted-RDMA test is qualified by the RXE run below, not by
counting an ignored test as executed. Formal cases remain NOT_RUN.

The caller receives `Cancelled / Timeout expired`, the exact tonic request-deadline representation in this run. Arbitrary cancellation is not accepted as proof of deadline. The registry eventually removes lookup; the client's per-call endpoint retires while the admitted server endpoint remains. After resume the original write completes with exact bytes. There is no automatic gRPC replay or client/server successful RPC payload count. All observed native IDs retire while the process is still alive.

A timeout is an unknown write outcome, not a rollback. This same-VM business-adapter proof does not establish physical DMA still pending, cross-VM network interruption, DFS deadline behavior or exceptional provider teardown.

| Level | Result / evidence |
| --- | --- |
| Focused local regression | [Actual posted/deadline audit](owner-deadline-v77-r4/cancel/audit.json), [GDB](owner-deadline-v77-r4/cancel/gdb.log), [runner](owner-deadline-v77-r4/runner.log) |
| Related regression | [Owner contracts](owner-deadline-v77-r4/owner-contract.log): 9 PASS / 3 explicit environment ignores; [healthy RXE](owner-deadline-v77-r4/owner-healthy.log) separately exercises one existing ignored case |
| Checker rejection checks | [Owner24](owner-deadline-v77-r4/owner-checker.json), [unchanged diagnostic20](owner-deadline-v77-r4/diagnostic-checker.json), [program](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-owner-rdma-deadline/audit-regression.py) |
| Compilation / static checks | [Compile](owner-deadline-v77-r4/compile.log), [affected Clippy](owner-deadline-v77-r4/clippy.log), [fmt](owner-deadline-v77-r4/fmt.log), [Linux Python compile](owner-deadline-v77-r4/python-compile.log) |
| Stage gate | No new full source gate. [v76 gate](../20261001-posted-rdma-cancellation/README.md) remains valid for its original inputs; production inputs/binaries unchanged. Fresh final candidate full gate remains due at batch/round closure |
| Formal acceptance | NOT_RUN / environment PREPARING |

## Identity and retained failures

The [executed test input](owner-deadline-v77-r4/test-input.rs.txt), [runner input](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-owner-rdma-deadline/owner-deadline-v77-r4/runner-input.py), [fixture binary hash](owner-deadline-v77-r4/binary.txt) and [runtime identities](owner-deadline-v77-r4/cancel/identity.json) bind this run. Only test/checker code changes; no production Rust/C/protocol, persistence format or architecture change.

The original attempts are retained: r1 assumed all deadlines have error kind DeadlineExceeded; r2 incorrectly rejected the transient poisoned lookup before asynchronous close; r3 expected an RPC success metric after its awaiting RPC had timed out. These are fixture assertion errors, not evidence of a production defect. The final fixture requires actual native CQ completion and physical exact content separately from RPC success metrics.

The remaining fault matrix is tracked in [issues](../../issues.md); this local task does not hold round 1 open for all RDMA edges. AGENTS and handoff remain unchanged.
