# OwnerFs RDMA stage evidence

The production OwnerFs remote file path selects its configured transport.
NodeControl negotiates and closes authenticated Owner windows; OwnerFiles
carries file commands and descriptors. Owner resources and authority remain
separate from DFS and diagnostics. This is a development stage, not release
acceptance.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Final Linux source gate | PASS: 403 library, 65 interface, 4 shared-error, 9 local API, 5 real root FUSE tests | [Gate logs](owner-linux-r5/owner-full-r5/lib.log), [Clippy](owner-linux-r5/owner-full-r5/clippy.log), [contracts](owner-linux-r5/owner-full-r5/contracts.log), [FUSE](owner-linux-r5/owner-full-r5/fuse.log) |
| Five supported feature configurations | PASS; existing `quarantined_chunks` warning in configurations without DFS | [Owner-only](owner-linux-r5/owner-features-r5/owner-features.log), [DFS RDMA](owner-linux-r5/owner-features-r5/dfs-rdma.log) |
| Final frozen compilation inputs | 143 exact input hashes | [Inputs](owner-linux-r5/inputs-r5.json), [host match](owner-linux-r5/match-r5.log) |
| Original failures and related local regression | 47 distinct tests PASS in r2 | [Owner contract](owner-linux-r2/owner-local-r2/owner-contract.log), [control](owner-linux-r2/owner-local-r2/control.log), [data](owner-linux-r2/owner-local-r2/data.log), [peer](owner-linux-r2/owner-local-r2/peer.log) |
| Affected core integrations | r2: 74 OwnerFs, 125 DFS, 25 replication, 39 Meta contracts PASS | [OwnerFs](owner-linux-r2/owner-related-r2/owner-vfs.log), [DFS](owner-linux-r2/owner-related-r2/dfs-vfs.log), [replication](owner-linux-r2/owner-related-r2/replication.log), [Meta](owner-linux-r2/owner-related-r2/meta-contract.log) |
| Owner registry without RDMA feature | 9 tests PASS in r2, including forged-scope lookup/close | [Log](owner-linux-r2/owner-control-no-rdma-r2/test.log) |
| Final actual Owner RXE | Two tests PASS, raw RPC and production OwnerPeerClient | [Full payload log](owner-rxe-r5/test.log), [resource observer](owner-rxe-r5/result.json), [binary identity](owner-linux-r5/owner-binary-r5/binary.sha256) |
| Native RDMA regression | Five actual RXE tests PASS under original native-r2 identity | [Log](native-r2/module/test.log), [resources](native-r2/module/result.json), [source/binary hashes](native-r2/identity.sha256) |
| Static independent review | No identified remaining blockers in reviewed paths | [Scope and findings](review.md) |

Counts overlap between local, integration and complete gates; they are not
added into one total. The library gate retains two environment ignores. The
interface run retains five FUSE and two RXE ignores; the separate privileged
FUSE and final RXE runs execute those seven tests. Formal cases stay NOT_RUN.

## What the RXE fixture proves

Each test writes 4 MiB + 17 bytes in five windows, performs file sync and close,
cold reopens with prefetch disabled, reads and compares every byte and BLAKE3,
then checks EOF. RDMA completion logs record 8,388,642 bytes in each direction
across both tests. File write uses server RDMA READ; file read uses server RDMA
WRITE. Inline payload is absent from RDMA file requests/replies. Both endpoints
run in one ARM64 Linux VM over its actual RXE provider with gRPC control.

The PID/thread observer sees positive QP/CQ/MR/PD/context ownership and none
retained by the observed test process after exit. This does not prove cleanup
while DMA is pending, exceptional provider teardown, production Node/FUSE
wiring across VMs, resource soak or loaded-provider identity. Existing native
teardown may retain memory if a provider cannot safely destroy QP/MR.

The client shares a 64-window admission limit per Node factory and holds each
permit through endpoint teardown. Required RDMA rejects explicit absence and
forbidden prefetch; Auto falls back only before dispatch on typed absence.
Capacity, authorization, protocol, checksum and unknown completion failures
never trigger replay through gRPC. Negotiation and transport close currently
add two control RPCs per file window; throughput qualification is open.

## Preserved failures and reuse

- [Original malformed-plane failure](owner-original-failure/test.log), exit
  101: the original handler accepted an unnegotiated descriptor and wrote three
  inline bytes. The new regression is the only changed input in that replay.
- [Original prefetch bypass](prefetch-original/test.log), exit 101: the r1
  client returned six inline bytes in required RDMA mode with no device.
- [Native first-candidate assertion failure](native-r1/run/test.log): product
  local buffer access was rejected correctly, but the test assumed a different
  error string. The corrected test preserves rejection and both transfer
  directions; no native product error text changed.
- [Raw Owner RXE r1 failure](owner-rxe-r1/test.log): a full final read returned
  `eof=false`, consistent with the existing short-read contract. The corrected
  fixture retains explicit zero-byte EOF, exact payload and digest checks.
- [First Clippy failure](owner-linux-r3/owner-full-r3/clippy.log): redundant
  Copy clones and collapsible conditions. [Second Clippy failure](owner-linux-r4/owner-full-r4/clippy.log): a redundant test default initializer. Neither
  attempt reached the remaining Rust test gate; the final r5 gate executes it.

Native-r2 is reused because the native test hash and its transport/error/build
inputs remain unchanged. Owner r2 integration/non-RDMA results retain their
original identity; r3-r5 changes narrow imports and make equivalent lint-only
edits. Final r5 library/contracts repeat affected all-feature tests and final
RXE rechecks actual bytes. Earlier intermediate runs are not relabeled r5.

The [Linux audit](https://github.com/lelezi257/dms/blob/e925c5bcf0408851ebfa08a59df29953374da9e9/development/evidence/20261001-owner-rdma/audit.py) validates the retained receipts, hashes, counts and
RDMA byte totals. Build work temporarily stopped idle C; [restored topology](environment/topology-restored.txt)
and [fresh C boot/RXE observation](environment/c-restored.txt) show ctl/A/B/C
running and build stopped. The new C observation does not qualify old transfer
evidence for that boot. No handoff or AGENTS change is part of this batch.
