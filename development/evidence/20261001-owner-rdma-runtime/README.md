# Production OwnerFs across two Linux VMs

This short integration uses the frozen v71-r5 product, memory Meta and separate
required-RDMA OwnerFs mounts on A and B. No Rust or protocol input changes.
The previous [source gate](../20261001-owner-rdma/README.md) is reused under its
original identity; deployment and probe changes receive fresh Linux validation.

## Results

| Check | Result | Evidence |
| --- | --- | --- |
| Linux probe regression | 7 PASS and compilation | [Probe tests](script-tests-final.log) |
| Evidence rejection checks | 7 PASS | [Audit tests](audit-tests.log), [validator](audit.py) |
| Initial remote write/sync/close | 4 MiB + 17 bytes, exact physical Home file | [B write](write-b-r3.json), [A storage](collect-a-after-write.json) |
| Cold reopen after B restart | Full content, length and EOF match | [Restart](restart-b.log), [B read](read-b-cold.json), [A DMA](collect-a-after-read.json) |
| 4 KiB patch and second cold restart | Full patched content and EOF match | [Patch](patch-b.json), [restart](restart-b-after-patch.log), [read](read-b-patched-cold.json) |
| Actual A server verbs | READ 4,198,417 bytes; WRITE 8,388,642 bytes | [A final](collect-a-final.json) |
| Idle resource snapshots | No QP/CQ/MR/PD/context owned by observed Node tasks | [A](collect-a-final.json), [B](collect-b-final.json) |
| Identified normal stop | Both Nodes and Meta exit 0; mounts absent; observed task resources absent | [A stop](stopped-a.json), [B stop](stopped-b.json) |
| Exact reused inputs/binaries | 143 source hashes/toolchain match; unstripped and stripped artifacts recorded | [Build inputs](build-inputs.json), [host match](host-compile-match.log), [artifacts](artifacts.txt) |

The [final Linux audit](audit-report.json) binds these observations and retained
files. Local regression and this stage integration do not satisfy the 69 formal
cases. The environment lock remains PREPARING.

## Flow and identity

A runs memory Meta and Node, B runs Node. A creates `owner-rdma-v72`; management
REST confirms Home A and its serving session. B writes `data.bin` through its
production FUSE mount, calls `fdatasync`, `fsync` and close. B Node is then
normally stopped and restarted with the same binary/config, replacing its
client and mount caches. A and Meta stay running. B cold reads every byte, then
patches 4 KiB and syncs. A second B restart precedes the full patched cold read.

A physical `data.bin` matches both payload states. B has no physical Home file.
Distinct boot IDs, PID/start ticks, executable/config hashes, mounts and exact
controller launch/exit receipts bind the process roles. A's native CQ logs prove
file write used RDMA READ and file read used RDMA WRITE. Each actual window is at
most 1 MiB; FUSE may split a 1 MiB application write into smaller callbacks.
Negotiation/probe messages are separate from counted file-byte completions.

Runtime directories remain preserved:

- A: `/mnt/lima-afsadata/afs-delivery/owner-rdma-v72-a`.
- B: `/mnt/lima-afsbdata/afs-delivery/owner-rdma-v72-b`.
- Ports: A 18780–18783, B 18784–18785.

Both new Nodes and Meta are stopped. Historical fixtures and services are kept.
The build VM was briefly started only to strip/copy artifacts; it was stopped
before runtime IO, and idle C was restored with its new boot/RXE observation.

## Failures and limits

Original [create](create-a.stderr) and [write](write-b.stderr) failed with HTTP
502 during the management query because VM proxy environment was inherited.
The file write was not dispatched in that failed attempt. The probe now makes
this private-cluster query with an explicit empty proxy handler. The original
workspace creation succeeded and is preserved; `home` verifies it without
creating another workspace. No product code changed for this probe repair.

The first [audit failure](audit-r1.stderr) assumed a nonexistent client metric
label. The second [audit attempt](audit-r2.stderr) used an incorrect path to the
retained input manifest. Their empty reports are retained as failures. The
corrected validator checks available client `open` and server timing plus exact
server file DMA totals; seven rejection checks prevent missing DMA, false cold
restart, wrong Home, changed physical bytes or retained idle resources from
passing. The [intermediate valid audit](audit-r3.json) retains its original scope.

Owner RDMA client read/write currently bypass their existing RPC timing metric,
and Owner payload bytes have no dedicated Prometheus counter. This is an
observability gap for a later product fix. Actual completion logs establish the
payload proof here. Earlier expected root lookup and unsupported xattr diagnostic
errors remain in A's full log; no transport error is claimed to have been erased.

Idle/post-exit snapshots do not prove peak bounds, posted-DMA cancellation or
exceptional provider teardown. These restarts clear B's view only; they do not
qualify Home failure, Meta restart, disk loss, performance, 8 GiB or soak.
Neither AGENTS nor the user-controlled handoff document was refreshed.

## Reproduce selected checks

Run [owner-rdma-runtime-v72.py](owner-rdma-runtime-v72.py) only as root in the
declared ARM64 Linux guests. `prepare` rejects existing runtime directories,
busy ports, non-ext4 storage, insufficient reserve or inactive RXE. Copy the
identified Linux artifacts and unchanged `afs-processctl`, then use its existing
`start all` on A and `start node` on B. Run A `create`, B `write`, restart B,
B `read`, B `patch`, restart B, B `read-patched`; collect after each phase and
stop only these identified runtime paths. `stopped` consumes the final collection
saved as `/tmp/owner-v72-final.json` and verifies matching exit receipts.

For retained evidence, run `python3 -m unittest -v test_audit` and
`python3 audit.py <evidence-dir> <v71-inputs-r5.json> <probe-path>` in Linux.
The existing directories are evidence fixtures, not safe fresh deployment paths.
