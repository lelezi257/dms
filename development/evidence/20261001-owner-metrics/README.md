# OwnerFs transport observability

Owner remote read/write now participate in the existing client duration metric
for gRPC, required RDMA failures and Auto fallback. The bounded
`afs_ownerfiles_payload_bytes_total` counter records successful logical bytes
by endpoint, direction and selected transport. File paths and operation
identities are not metric labels. RPC schemas and native provider inputs are
unchanged.

## Results and validation scope

| Level | Result | Evidence |
| --- | --- | --- |
| Original failure | Missing-device RDMA read duration count was 0 instead of 1, exit 101 | [Original regression](original-regression/owner-metrics-v73-original/timer.log) |
| Local feedback | 51 distinct related Owner/control/data/peer regressions pass; original timer failure covered in peer tests | [Peer](owner-metrics-v73-r1/local/peer.log), [Owner](owner-metrics-v73-r1/local/owner-contract.log), [data](owner-metrics-v73-r1/local/data.log), [control](owner-metrics-v73-r1/local/control.log) |
| Frozen r2 source gate | 407 library, 65 contract, 4 shared-error, 9 local API and 5 actual root FUSE tests pass; formatting, strict workspace/all-target/all-feature Clippy, five feature checks and binaries pass | [Full gate](owner-metrics-v73-r2/full/lib.log), [contracts](owner-metrics-v73-r2/full/contracts.log), [Clippy](owner-metrics-v73-r2/full/clippy.log), [features](owner-metrics-v73-r2/features/owner-rdma.log) |
| Actual RXE client integration | 4 MiB + 17 bytes, sync/cold read/EOF and client metric assertions pass on r2 | [Native integration](owner-metrics-v73-r2/native-owner.log) |
| Production cross-VM integration | B remote write/sync/close, two normal B restarts, full cold reads and a 4 KiB overwrite pass | [Write](runtime/write-b.json), [cold read](runtime/read-b-cold.json), [patch](runtime/patch-b.json), [patched cold read](runtime/read-b-patched-cold.json) |
| Probe and evidence checks | Seven probe regressions and ten evidence checks pass | [Probe](runtime/script-tests-final.log), [audit regressions](runtime/audit-tests-final.log), [runtime audit](runtime/audit-report.json) |
| Formal acceptance | 69 NOT_RUN; ENV PREPARING | No formal case is promoted by this batch |

Two existing library environment ignores remain. Five FUSE tests ignored by the
unprivileged contract invocation run separately as root. The two environment
dependent Owner contract tests are not relabeled: this batch explicitly runs
the production OwnerPeerClient RXE case; the unchanged raw-RPC case retains its
previous qualified identity. Single-feature checks retain the existing
`quarantined_chunks` dead-code warning; strict all-feature Clippy passes.

Small edits receive original-failure and affected-module feedback. The complete
source gate runs at this coherent batch closure. No full POSIX, performance,
8 GiB or soak matrix is repeated. Earlier source/runtime evidence remains
bound to its original inputs; this runtime uses freshly built r2 binaries.

## Logical byte accounting and actual DMA

Successful logical read/write lengths are counted after response shape and
checksum validation. Short I/O counts its actual length; EOF adds zero. Failed
or malformed completions and client cache hits add no successful payload.
Client timing includes negotiation, transfer completion checks and close.
Server completion and client acknowledgement can differ after reply loss or
retry; these metrics are not physical wire-byte counters.

| Process observation | RDMA write bytes | RDMA read bytes |
| --- | ---: | ---: |
| B initial writer | 4,194,321 | 0 |
| B cold-reader/patcher after first restart | 4,096 | 4,194,321 |
| B cold reader after second restart | 0 | 4,194,321 |
| Home A throughout the flow | 4,198,417 | 8,388,642 |

All corresponding gRPC file-payload counters are zero. The three B process
identities are distinct and its process-owned metrics reset at each restart.
Home A and memory Meta keep their identities. A's native CQ logs independently
record RDMA READ 4,198,417 bytes for remote writes and RDMA WRITE 8,388,642 bytes
for remote reads. Exact content, length, EOF and physical Home file hashes
match; B has no physical Home file. The [audit](runtime/audit.py) checks these
facts together rather than inferring DMA from a counter alone.

The [143 frozen inputs](build-inputs.json) and [binary hashes](owner-metrics-v73-r2/artifacts.txt)
bind the Linux build to the probes. Stripped Node SHA is
`e4647269edc913e9ae04f5df8089c1ef96799274e8e7449e014fedc8530fae49`;
Meta SHA is `9bd2fa7dc973590340d8b7674edcb27f9f41db1b475f930525e9e08c8fe6a930`.
The build VM is stopped before runtime I/O; ctl/A/B/C are running and C's RXE
is restored. Tested bytes reside on A/B guest ext4.

## Failures retained and limits

Original v72 inputs are recorded before grafting only the new regression. The
grafted test fails with the missing timer assertion; it does not use the fixed
production implementation. r1's first batch gate stops at formatting; Linux
formatting produces r2 and its complete fresh source gate passes. An initial
build RXE check fails because `ibv_devinfo` is absent; installation and final
device observations are preserved. C's first restore command refers to an
absent file; copying the unchanged configuration script restores RXE. The first
audit invocation uses the parent transfer directory and cannot import the test;
the corrected directory passes. These are separate recorded failures, not
reasons to repeat unchanged product tests.

The first host hashing command uses zsh's special `path` variable and cannot
locate the hash tool; the batch audit rejects the resulting missing inputs.
Using an ordinary variable and an absolute hash-tool path records all 143
inputs, which match the qualified Linux snapshot. The exact r1 transfer archive
is also [hashed in Linux](r1-archive-inputs.json); that derived archive record is
distinct from the frozen r2 build manifest.

No new Node/Meta is left running: matching normal exit receipts, absent mounts
and idle/post-exit resources are retained. A/B runtimes and prior fixtures are
preserved at `owner-metrics-v73-a` and `owner-metrics-v73-b`, ports 18880–18885.
Idle snapshots do not establish peak/cancellation/exceptional teardown bounds.
Home failure, Meta restart, disk loss, backend parity and performance remain
outside this short healthy flow. AGENTS and the user-controlled handoff are
unchanged.

## Reproduce selected checks

Run [source-gate.sh](source-gate.sh) only in the separate Linux build VM against
a frozen source tree and new output directories. `local` runs affected RPC
regressions; `full` and `features` supply the batch gate. Explicitly run the
ignored `ownerpeerclient_rdma_large_write_fsync_cold_read_roundtrip_preserves_payload`
test with `AFS_TEST_RDMA_DEVICE=rxe0` for actual client integration.

Run the [runtime probe](runtime/owner-metrics-runtime-v73.py) as root in A/B.
Prepare fresh ext4 paths, install the recorded binaries and unchanged process
controller, then create at A and perform B write/read/patch/read-patched with
two normal B restarts. Collect each process incarnation before restart. Save
each final collection as `/tmp/owner-metrics-v73-final.json` before normal stop;
`stopped` validates exact matching process/config/exit receipts. Existing
directories are preserved fixtures and must not be overwritten.

Run `python3 -m unittest -v test_audit` and `python3 audit.py <runtime-dir>
<qualified-inputs.json> <probe-path>` in Linux for retained runtime validation.
The runtime audit establishes the short integration; the separate source-gate
records establish the source gate. Neither establishes formal acceptance.
