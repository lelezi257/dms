# OwnerFs RDMA and native lifetime batch

Status: STAGE_GATE_PASS. This is a development batch, not formal acceptance.

## Failure and scope

The production OwnerFs factory constructs an inline gRPC client regardless of `data_mode`. Owner read/write handlers ignore `DataPlane`. Existing five RXE diagnostic tests pass, but do not exercise the OwnerFs file path or a posted-DMA cancellation.

Preserve the existing Node, FUSE, OwnerFs and four RPC module boundaries. Add authenticated Owner-specific transport negotiation and close in NodeControl; file requests remain in OwnerFiles. An independent registry binds sessions to exact authenticated root and process authority. File read/write permissions remain per-operation checks.

## Checks by impact

1. Native transport test changes: affected probe tests, actual RXE READ/WRITE full-payload comparisons, timeout rejection and bounded repeated lifecycle checks.
2. Owner RPC additions: compile all callers and feature configurations early; targeted protocol/authorization/window/checksum/required-mode tests; real OwnerFs create/write/sync/close/reopen/EOF tests.
3. Related transport batch: affected DFS completion integrity and existing RDMA lifetime regressions, OwnerFs and DFS integration/authentication checks. No automatic replay of an uncertain write on another transport.
4. Batch closure: full Linux source gate for the final frozen Rust/protocol inputs. The v64 gate cannot qualify changed Rust/protocol inputs. Formal POSIX, 8 GiB, performance and long stability matrices remain scheduled separately.

Record local regression, stage gate and formal acceptance results separately. Do not refresh `docs/handoff.md`.

## Original evidence

`original/identity.json` verifies all 143 cached v64 compilation inputs against their original manifest. The unchanged harness compiled on Linux and ran five explicitly selected ignored RXE diagnostic tests on A with no failures. It establishes only its declared diagnostic coverage; no before/after resource capture was taken for that run.

Build VM use temporarily stops C after a read-only observation showing no AFS process or mount. C must be restored, RXE reconfigured and current boot identity observed after build work; its previous boot evidence cannot be reused as a fresh observation.

## Native short regression

The first candidate retained four passing tests and one failed assertion: poisoned local buffer access was correctly rejected, but the regression assumed the native error message contained `poisoned`. The existing native buffer methods report `bounds/state`. No product behavior or error text was changed to satisfy that assertion.

The corrected test establishes valid local buffer access before the timeout and rejection afterwards, including both transfer directions. Linux compilation and formatting pass; the original failing test and the five-test affected RXE module pass. Full 4 MiB payloads are compared after both READ and WRITE. Four 64 KiB create/probe/transfer/drop cycles return process uverbs FDs to their baseline.

The PID-scoped observer sees two QPs/CQs/MRs/PDs/contexts during each observed run and none retained by the test's observed thread identities after process exit. This proves the observed process cleanup, not exceptional provider teardown or resource release while a cancelled DMA is still pending. Original diagnostics, failed candidate and corrected results remain separate under `original/`, `native-r1/` and `native-r2/`.

## Owner original failure and review boundaries

`owner-original-failure/` reproduces the defect on the original product: a malformed RDMA-plane write with no negotiated session, a nonzero buffer offset and inline bytes returned `written=3`. Only the new test input differs from the retained 143-input original manifest; the other 142 product inputs match. The original failure is retained with exit 101.

The server/client candidate must reject that request before mutation, bind negotiation and cleanup to the exact authenticated root scope, and preserve short I/O. Required RDMA cannot silently use gRPC when a device is absent. Auto may fall back only for explicit transport absence before file command dispatch; unknown writes and protocol, authority or checksum errors are never replayed through gRPC.

Independent static review found asynchronous authority calls that could reach `GrpcRootMeta::block_on`, read allocation preceding negotiated size checks, and poisoning after endpoint lock release or cancelled waits. These findings require blocking authorization, preallocation bounds, and worker-side poisoning plus cancellation guards. Their repair is not validated by the previous native transport tests. Production client tests supplement raw control/data fixtures; raw RDMA tests alone cannot qualify factory/client selection.

## Owner candidate r1 results

Root's Linux snapshot matches all 143 retained input names against `host-inputs-r1.sha256`. The necessary all-feature contract compilation, 45 distinct selected local tests and five feature configurations pass. The original malformed-plane failure is included in the contract test set and is not counted twice. These are local results, not the final batch gate.

The real RXE two-test run retains exit 101 in `owner-rxe-r1/`: production OwnerPeerClient passes its large write/sync/reopen/read/hash flow; the raw fixture fails its assertion that a full final read must set EOF. The existing handler signals EOF on a short read, so a full read ending exactly at file length need not set it. Both payload transfer directions completed, but this failed run remains FAIL. The corrected fixture must retain the explicit zero-byte read at file length and full payload comparison.

Client review also requires ignoring unexpected inline prefetch outside gRPC mode and bounding local endpoint allocation before opening an MR. Permits must remain with detached blocking workers until endpoint teardown. These changes require a fresh frozen candidate and affected checks; r1 cannot qualify the changed client. A complete Linux source gate and actual RXE rerun remain pending. Cross-VM production Node/FUSE Owner RDMA and posted-DMA cancellation are still outside this short fixture's proof.

## Final stage checkpoint

The retained r1 section describes that intermediate candidate. Final r5 passes
the complete Linux source gate and both actual Owner RXE payload fixtures.
The results, preserved failures, exact reuse scope and remaining release work
are summarized in [README.md](README.md). C is restored with a fresh boot/RXE
observation; build is stopped. Formal ENV qualification and all 69 formal cases
remain unchanged. No handoff refresh was performed.
