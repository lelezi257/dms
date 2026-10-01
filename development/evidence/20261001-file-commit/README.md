# Exact FileVersion commit and lost reply

## Result and scope

**Stage gate PASS:** the frozen v64-r1 Linux ARM64 candidate passes 190 related
regressions, a real gRPC FileVersion reply-loss integration and the complete
source gate. [Structured results](linux/result.json),
[143 compile inputs](linux/compile-inputs.json) and
[actual Linux identity](linux/linux-identity.json) bind this evidence.

This is a memory-backed, plain loopback, R1 development fixture using real Meta
handlers, the real gRPC adapter, product `DfsChunkStore` placement and guest local
disk. It proves same-process close/reopen, not crash recovery, durable backend,
TLS/RDMA fault matrices or formal REL-04. All 69 formal cases remain NOT_RUN;
the environment lock remains PREPARING. Existing deployed v62 runtimes retain
their own identities. Handoff is unchanged.

## Commit identity

Meta binds the complete `CommitFileVersion` body to its recorded operation using
the existing namespace digest wrapper. Exact replay returns the original inode;
changed same-ID version/layout is rejected without changing head, version,
layout or outcome. Both early replay and transaction-condition replay validate
the digest. Old direct inode outcomes remain readable, but cannot authorize a
retry whose original body cannot be proven. Upgrade/migration remains deferred.
No public RPC, module, dependency or result variant was added.

The [original Meta regression](linux/original/meta-replay.log) fails with exit
101 against pre-fix production behavior. The [focused rerun](linux/meta-fix-r1/original.log)
and [39 Meta contracts](linux/meta-fix-r1/contracts.log) pass after the fix.
[Store regressions](linux/meta-fix-r1/store-module.log) pass 27 tests with one
existing Redis environment ignore. A [wrong module selector](linux/meta-fix-r1/meta-module.log)
executed zero tests and is not a PASS; the [corrected selector](linux/meta-fix-r1/meta-module-corrected.log)
executes 20. [Two additional compatibility/replay regressions](linux/meta-fix-r2/meta-module.log)
bring the affected Meta module to 22.

## Actual reply-loss flow

The [wire integration](linux/wire/wire.log) checks these states:

1. A writes `alpha`; its first sync commits the inode, FileVersion, LayoutRoot
   and outcome in real Meta. A test-only layer delays the reply after handler
   completion beyond the client deadline. Node retains the complete pending
   request, frozen write sequence and replica receipt.
2. Exact confirming replay reaches a second post-handler response gate. The
   four stored records remain identical. Same-inode write, resize and sync
   cannot complete or mutate pending state; no chunks are prepared again.
3. Inode B in the same `DistributedFs` and write-state table writes, syncs and
   reads `bravo` while A remains held. A's permitted local read returns `alpha`.
4. Releasing the reply drains the queued work. Explicit sync, close-time flush,
   release and reopen return `alpha!\0`, length 7 and EOF. Waits are bounded.

Original [fixture compile errors](linux/wire-compile-failure/wire.log),
[endpoint timeout-code assertion](linux/wire-timeout-code-failure/wire.log) and
[invalid fixture replica receipt](linux/wire-invalid-fixture-receipt-failure/wire.log)
remain failed under their own input hashes. The last failure correctly found
that a mock-only LocalChunkStore adapter had not committed anything in Meta.
The passing fixture uses actual product placement and R1 receipts. It accepts
only the adapter deadline or the known endpoint `Cancelled / Timeout expired`
race, and still requires proof that Meta committed.

## Validation selection

Small fixture fixes reran only the wire test. The related batch then ran
[125 VFS tests](linux/local/vfs.log), [22 Meta tests](linux/local/meta-module.log),
[four Meta adapter tests](linux/local/rpc-meta.log),
[39 Meta contracts](linux/local/meta-contract.log), formatting and strict Clippy.

The final complete gate ran once at batch closure: [399 library PASS, two
existing environment ignores](linux/full/lib.log),
[58 contracts](linux/full/contracts.log), [four shared-error tests](linux/full/error.log),
[nine local API tests](linux/full/local-api.log) and
[five privileged real FUSE tests](linux/full/fuse.log). Formatting, strict
workspace/all-target/all-feature Clippy, supported feature checks and binary
build all exit zero. No-feature/OwnerFs-only checks retain the known unused
`quarantined_chunks` warning; all-feature strict Clippy passes. Redis/RXE library
ignores are not formal acceptance evidence.

[Runner](runner.sh) records command/log/exit per selection;
[static review](static-review.md) is separate from executed evidence.
Full POSIX, 8 GiB, comparison performance and long stability matrices remain
in their scheduled stages. Historical v63 results are not relabeled as v64.

