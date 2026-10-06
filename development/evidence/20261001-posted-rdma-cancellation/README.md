# Posted RDMA cancellation and normal resource release

The Linux RXE diagnostic exercises a real NodeData write. GDB stops only its
native worker after successful `ibv_post_send` and before CQ consumption. The
caller cancels its future and independently closes the session lookup. The
client refuses reuse, the lookup becomes stale, and both endpoints retain their
exact QP/MR/CQ/PD/context identities. After resume, the admitted write completes
with exact contents; those resource identities disappear while the test process
is still alive.

This proves posted and unconsumed work. It does not prove that physical DMA is
still pending. Cancellation has an unknown outcome; it does not roll back an
already admitted write.

## Validation levels

| Level | Result | Evidence |
| --- | --- | --- |
| Harness failures | Initial Rust E0716 and GDB event-loop stall retained; neither indicates a production defect | [Compiler](posted-cancel-v76-r1/compile.log), [initial test input](posted-cancel-v76-r1/test-input.rs.txt), [debugger](posted-cancel-v76-r2/cancel/gdb.log), [timeout events](posted-cancel-v76-r2/cancel/debugger.jsonl), [runner failure](posted-cancel-v76-r2/cancel-runner.log) |
| Local regression | Final posted-cancel case passes; default lifecycle1, existing actual RXE lifecycle5 and native probe5 pass:12 distinct cases including the new case | [Final GDB](posted-cancel-v76-r6/cancel/gdb.log), [exact resource audit](posted-cancel-v76-r6/cancel/audit.json), [default](posted-cancel-v76-r4/lifecycle-default.log), [lifecycle](posted-cancel-v76-r4/lifecycle-native.log), [native](posted-cancel-v76-r4/native-probe.log), [targeted Clippy](posted-cancel-v76-r4/local-clippy.log) |
| Checker regression | 20 checks include valid observed JSON and rejection of missing post, wrong operation, wrong QP/device, missing MR, retained MR without live PID, missing/duplicate completion and invalid content/replay claims | [Checks](posted-cancel-v76-r5/audit-regression.json), [test program](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-posted-rdma-cancellation/posted-cancel-v76-r5/audit-regression.py) |
| Final stage source gate | Library414/6 explicit environment ignores, contracts65, shared-error4, localAPI9, rootFUSE5; fmt, strict all-target/all-feature workspace Clippy, five feature configurations and binaries pass | [Gate runner](posted-cancel-v76-r5/gate.sh), [library](posted-cancel-v76-r5/full/gate/lib.log), [contracts](posted-cancel-v76-r5/full/gate/contracts.log), [FUSE](posted-cancel-v76-r5/full/gate/fuse.log), [Clippy](posted-cancel-v76-r5/full/gate/clippy.log) |
| Identity audit | 44 checks bind143 compiler inputs, final checker, real resource proof, test binary and unchanged production bytes | [Audit](posted-cancel-v76-r5/audit.json), [program](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-posted-rdma-cancellation/posted-cancel-v76-r5/audit.py), [inputs](posted-cancel-v76-r2/build-inputs.json), [binaries](posted-cancel-v76-r5/binaries.txt) |
| Formal acceptance | NOT_RUN; ENV PREPARING | No release case is promoted |

The default lifecycle suite explicitly ignores6 environment-dependent cases.
Five are executed separately with real RXE; the sixth is executed through the
external GDB runner. An ignored result is not counted as a pass. Multiple
successful debugger refinements execute the same new case and are not counted
as different cases.

## Resource proof

The final fixture uses process2947 and the stopped native worker2957. QPs51/52,
MRs34/35, CQs35/36, PDs35/36 and contexts34/35 remain after lookup close while the
worker is paused. The posted QP52 is one of the retained QPs. The drained
inventory contains none of those exact device IDs, including any record whose
creating thread might no longer exist. The completed payload is one4096-byte
RDMA READ; reuse produces no new file or data completion.

[Raw paused QPs](posted-cancel-v76-r6/cancel/closed-paused-qp.json),
[raw drained MRs](posted-cancel-v76-r6/cancel/drained-mr.json) and
[debugger events](posted-cancel-v76-r6/cancel/debugger.jsonl) accompany the result.
The checker deliberately supports the observed flat Linux iproute2 JSON;
unsupported schemas fail rather than being interpreted as empty inventories.

## Frozen evidence reuse

Only `tests/rdma_lifecycle.rs` changes among the143 compiler inputs compared with
[v75](../20261001-dfs-auto/README.md). No production Rust/C/protocol, disk format,
public API, dependency or runtime module changes. Fresh stripped Node/Meta
hashes exactly match v75. Existing OwnerFs/DFS healthy file and authority
integrations retain their original evidence identities; they are not rerun or
claimed as new fault qualification.

One full source gate runs at batch end. Subsequent checker-only refinements use
the unchanged Rust input/test binary identity and rerun the focused debugger
case and checker rejection tests. The final [executed checker identity](posted-cancel-v76-r6/cancel/identity.json)
records its own source hash and native checkpoint hash. Original runner versions
are retained beside each run. The unstripped fixture is retained in Linux at
`/home/lzc.guest/afs-build/artifacts/posted-cancel-v76/rdma-lifecycle`.

[Review](REVIEW.md) is file inspection, separate from runtime proof.
[Runner notes](runner-note.md) distinguish setup errors from product failures.

## Limits

This is real same-VM diagnostic NodeData transport on the Linux ARM64 build VM,
not a new OwnerFs/DFS fault deployment in the qualified four-VM environment.
Posted cancellation in those business adapters, actual network/hardware DMA
interruption, native completion timeout after data posting, exceptional
QP/MR destruction failure, admission peaks and long fault matrices remain open.
No file durability, performance or full POSIX claim follows from this fixture.
Formal69 cases remain NOT_RUN; the total delivery goal remains active. AGENTS and
handoff retain their original hashes.
