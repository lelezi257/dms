# Round 2 business network and physical capacity faults

**Two scoped fault integrations PASS; ROUND2 remains incomplete.** Formal status
is **69 NOT_RUN / ENV PREPARING**. This batch changes probes and evidence only;
production Rust, protocols, formats, modules, interfaces and dependencies are
unchanged. AGENTS and handoff remain unchanged.

## Business network interruption

The [existing async cohort](../20261001-round2-node-recovery/README.md) retains
central memory Meta and Home A. B is cold restarted before the fault. A dedicated
B OUTPUT rule drops only A's actual business RPC port `192.168.109.12:19982/TCP`.
It does not block unrelated runtimes, Meta, C or the RoCE link. A detached Linux
watchdog is armed for 90 seconds; normal finally cleanup removes the owned rule.
Automatic recovery after a killed fault controller is not tested here.

| Event | Observed behavior and why it matters |
| --- | --- |
| [Actual port isolation](network/b/jump-install.json) | [DROP counters](network/b/iptables-during.json) show nine packets/540 bytes during a 33-second interval. Direct TCP succeeds before and after; it times out during the rule. This is business-port proof, not an environment-only probe port |
| [B Owner open](network/b/owner-blocked.json) | EHOSTUNREACH, zero bytes, 10.03 seconds. Loss of the Home route produces an explicit bounded failure |
| [B DFS read](network/b/dfs-during.json) | Exact 4 MiB +17 content and EOF while isolated. B already has retained durable chunks from the preceding recovery batch; this proves unaffected local DFS access, not new remote-source failover |
| [C Owner read](network/c/round2-network-v79b-c-during.json) | Exact content during the rule window. The scoped B route fault does not disable the other Home route |
| [B recovery](network/b/owner-recovered.json) | Same Node incarnation reads correct Owner content in 0.105 seconds after removal; DFS also remains correct. [Rollback](network/b/rollback.json) and [firewall dump](network/b/iptables-after.json) show no owned chain remains |

The original A/C/Meta identity observations are reused from the preceding
published packet and compared with fresh final observations. B's actual
before/cold/final identities are recorded in this packet. No memory-Meta
persistence or in-flight RDMA completion claim follows from this TCP fault.

## Physical ENOSPC and retained committed data

A separate memory Meta runs on ctl:20080 and a Node on A:20082, using the same
production binaries, mTLS, independent OwnerFs/DFS mounts and gRPC. The Node's
data resides on a bounded **128 MiB logical loop/ext4 filesystem**, backed by a
file on the locked guest virtio/ext4 data volume. Logs, process identity and
evidence remain outside the fault filesystem. This is a supplemental development
fixture, not qualification of the formal device or fault matrix.

The original N2/M1/local-required policy remains configured. Only A participates
in this fixture, so it does **not** prove two live replicas or asynchronous
repair. Required local persistence must succeed before a durable acknowledgement.
`mkfs.ext4` discards backing blocks; the image is not claimed to remain physically
preallocated. The filler itself is measured: 106,205,184 written bytes and
106,209,280 allocated bytes, successful filler fsync, real allocation ENOSPC and
zero usable `f_bavail`. ext4 retains metadata/emergency blocks in `f_bfree`.

| Event | Observed behavior and why it matters |
| --- | --- |
| [Initial watermark](capacity/a/evidence/seed.json) | Both 8 KiB files complete write, fdatasync, fsync and close; content/length/EOF are verified |
| [Actual capacity exhaustion](capacity/a/evidence/fill.json) | A non-sparse filler in the data filesystem returns ENOSPC. A 180-second detached watchdog removes only that disposable filler if the controller disappears; watchdog failure cleanup is not separately qualified |
| [Owner append](capacity/a/evidence/fault-ownerfs.json) | Write immediately returns ENOSPC. Subsequent sync/close succeed because the rejected append admitted no data; those successes are not a claim that the rejected bytes persisted |
| [DFS overwrite](capacity/a/evidence/fault-dfs.json) | Write admits 8 KiB to memory, while fdatasync, fsync and close each return ENOSPC. A buffered write success is distinguished from durability |
| [Same-mount dirty read](capacity/a/evidence/post-error-kill.json) | The accepted dirty overwrite remains locally visible, as required by the current consistency contract. It is not a committed FileVersion |
| [Cold reload while still full](capacity/a/evidence/cold-committed-read.json) | After identity-bound SIGKILL and installed-controller start, a new Node incarnation reads the original committed 8 KiB from both mounts. The fault still has zero usable space. This proves retained acknowledged content; the audit does not directly inspect Meta head/layout IDs |
| [Capacity recovery](capacity/a/evidence/recover.json) | Removing only the owned filler restores space. New writes, both syncs and close succeed with exact new content |
| [Later normal restart](capacity/a/evidence/verify.json) | Both files retain the new successful content and EOF in another Node incarnation |
| [Cleanup](capacity/a/evidence/cleanup.json) | Capacity Node and Meta stop normally; filler is absent, loop filesystem unmounted/auto-detached, physical data remains inspectable, outer guest data reserve stays above 4 GiB |

Capacity Meta's actual PID/start ticks/boot/SHA remain constant across the Node
fault. The original four-node cohort also remains intact. Meta restart durability,
device EIO, disk destruction, RoCE interruption and complete failure matrices
remain unproved by this batch.

## Verification and retained preparation limits

[Linux semantic audit](audit.json) passes 64 checks; [audit regressions](audit-regression.json)
pass ten checks, including rejection of false sync success, wrong port, no actual
DROP, changed authority, sparse filler, incorrect cold watermark and bad recovered
bytes. [Reuse verification](reuse-verification.json) checks all 143 compiler inputs
and the protected documents. The unchanged completed source gate is reused;
no new Rust gate, full POSIX, performance matrix, 8 GiB or soak is claimed.

[Preparation observations](preparation-observations.json) summarize unsuccessful
preparations separately from product behavior. Actual per-operation fault records
and runtime logs remain in the packet. The original capacity worker versions are
retained under `capacity/`; corrected replay scripts live under `reproducers/`.
The corrected replay script is syntax checked; it is not relabeled as the worker
that produced every original observation.

The ctl reserve guard initially refused setup. Three inactive package extraction
trees were copied to guest root ext4 with content/mode/owner/mtime/xattr verification
and original-path symlinks; [migration proof](capacity/ctl/round2-capacity-v79-ctl-reserve.json)
raises its reserve from 4,207,206,400 to 4,404,584,448 bytes without moving live state.
Initial fixture predicates incorrectly required zero `f_bfree` and only committed
bytes for a same-mount dirty read. The installed controller reports prior crash
exit137 when `restart` first stops that lifecycle; `start` correctly handles stale
crash recovery. An explicit loop detach was redundant after autoclear unmount.
These observations do not establish production defects or change the accepted
contract.

The cold operations were executed from bounded Linux inline commands; equivalent
[replay helpers](reproducers/capacity-cold.py) are reconstructed from those commands:
obtain the
worker's actual `/proc` identity, retain the dirty read, use `os.pidfd_open` plus
`signal.pidfd_send_signal(SIGKILL)`, run installed `afs-processctl start node`,
then call `check_read(path, BASE)` while `f_bavail == 0`. Normal recovery invokes
the worker's `recover`, followed by `restart`, `verify` and `stop`; the ctl worker
records authority identity and stops Meta. Replay helper syntax is checked; a
full rerun under its reconstructed filename is not claimed. Retained raw records,
process logs and worker code bind the scoped proof. Complete formal-driver fault
and rollback qualification remains due.

Next ROUND2 priorities: actual storage EIO/corruption, exact unknown-commit
integrations and remaining business RDMA exceptions. See [issues](../../issues.md).
