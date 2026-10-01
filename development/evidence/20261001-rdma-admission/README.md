# RDMA server admission ownership

Server admission reserves a slot before native allocation and retains it with
the endpoint Arc until its last owner drops. Session close/TTL removes lookup
authority and does not release a busy worker's slot. Each registry keeps its
existing limit of 64; Owner and DFS/diagnostic budgets remain separate.

## Validation levels

| Level | Result | Evidence |
| --- | --- | --- |
| Original failure | Old main admits session65 while64 closed-session endpoints are retained; exit101 | [Failure](rdma-admission-v74-original/regression/original.log), [graft](rdma-admission-v74-original/graft.diff) |
| Local regression | 55 distinct cases pass: control9, retained-endpoint RXE1, data22, peer14, Owner contracts9 | [Control](rdma-admission-v74/local/control.log), [native regression](rdma-admission-v74/local/original.log), [data](rdma-admission-v74/local/data.log), [peer](rdma-admission-v74/local/peer.log), [Owner](rdma-admission-v74/local/contracts.log) |
| Final source gate | Library410/3 existing-or-explicit environment ignores, contracts65, shared-error4, localAPI9, rootFUSE5; fmt/strict workspace all-target all-feature Clippy/five feature configurations/binary build pass | [Library](rdma-admission-v74-r2/full/gate/lib.log), [contracts](rdma-admission-v74-r2/full/gate/contracts.log), [Clippy](rdma-admission-v74-r2/full/gate/clippy.log), [features](rdma-admission-v74-r2/full/features/dfs-rdma.log) |
| Final affected integrations | Explicit retained-endpoint regression, DFS real replica/read/auth/retry, production Owner4MiB+17/sync/coldread/EOF, five lifecycle cases pass | [Admission](rdma-admission-v74-r2/original/original.log), [DFS](rdma-admission-v74-r2/native/dfs.log), [Owner](rdma-admission-v74-r2/native/owner.log), [lifecycle](rdma-admission-v74-r2/native/lifecycle.log) |
| Audit | 39 checks bind143 compiler inputs, original failures, test counts, actual DMA and stripped binaries | [Audit](rdma-admission-v74-r2/audit.json), [script](audit.py) |
| Formal acceptance | NOT_RUN; ENV PREPARING | This batch does not promote any formal case |

Small changes receive failure/module/compile feedback. The final full source
gate is completed at this coherent batch boundary. Full POSIX, performance,
8GiB, backend matrices and soak retain their planned acceptance stages.

## Actual transport evidence

The DFS fixture records two4MiB RDMA READs for a durable replica write and its
exact retry, then a75,000-byte RDMA WRITE for authorized range read. It rejects
a forged grant; gRPC file payload is zero. The production Owner client completes
4MiB+17 bytes in each native direction with content, sync, cold reopen, EOF and
metric assertions. All run on Linux ARM64 RXE, guest storage, the frozen final
source, and real product handlers. This batch does not deploy new A/B processes.

Native admission proves64 retained server endpoints block the65th negotiation
and last-owner release permits negotiation again. Concurrent reservation and
failed allocation recovery tests pass. [Static review](REVIEW.md) finds zero
blocking issues; it is separate from executed evidence.

## Identity and retained failures

The original [143 inputs](rdma-admission-v74-original/before-graft-inputs.json)
are main567ef0f before grafting only the regression. [Final inputs](rdma-admission-v74-r2/build-inputs.json)
match the product tree. r1 native/local results stay bound to r1. Its full gate
stops at Clippy101 for a must-use test permit. [r1→r2](rdma-admission-v74-r2/r1-r2.diff)
changes only that test's binding/drop assertion; the audit proves production
text identical. r2 reruns the changed control regression before its complete
fresh gate and selected native integrations. No failure is hidden or waived.
Linux formatting before freezing is retained with before/after hashes.

[Runner notes](runner-note.md) retain a local outer-shell exit1 despite its
cargo exit0, a command-render typo, and a premature diagnostic log read. Final
independent gate and native receipts provide the qualified results.

Stripped Node SHA: `32b3030e131215a2f676d98cccadf005da72b8da33eea9eb0c19981ee49db435`.
Stripped Meta SHA: `edd27444f109d5176d71c393b586a934089b421bd92dfe28d7f6df93537bc9fd`.
Both reside on afs-build in `afs-build/artifacts/rdma-admission-v74-r2`;
the [manifest](rdma-admission-v74-r2/stripped-binaries.txt) and audit verify them.
The audit requires retained Linux source snapshots/artifacts and the product
tree, rather than inferring compiler identity from Git HEAD.

## Limits

The cancellation lifecycle fixture uses a delayed fake RPC, not posted data
DMA. Cancellation after WQE posting, exceptional provider destruction leaks,
peak retained native resources, TTL during active DMA, peer/process failures
and long-run resource growth remain unqualified. The after-run kernel summary
reports pd1/cq1/qp1/mr0/ctx0; it does not establish complete resource reclamation.
Admission bounds AFS-owned live endpoints and pending allocations and cannot
prove recovery of resources a broken native Drop intentionally retains.

Public endpoint representation and Default initialization changes are recorded
for final review. No RPC schema, disk format, module or dependency changes.
DFS Auto factories still select gRPC and are a separate pending correction.
AGENTS and docs/handoff.md retain their original hashes. Acceptance standards
and69 NOT_RUN remain unchanged; the overall delivery goal stays active.
