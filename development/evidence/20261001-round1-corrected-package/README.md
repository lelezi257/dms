# Round 1 normal system flow

**Mainline flow PASS.** The final Linux source stage gate is reused under its
unchanged 143 compiler inputs. Formal acceptance remains **69 NOT_RUN**, ENV
**PREPARING**. This closes the scoped healthy first round and starts the major
fault/recovery round; it does not establish delivery acceptance.

## Candidate and proof boundaries

The corrected archive is SHA256
`e13bc12e2d51ea70287a935c4f2a92b6b5313979ac94a2b080e0ec89fbaac8b5`,
manifest source `370cba91a15999a7edd7edb644a852c2b2dfd89b`.
All eight [R1](r1/a/evidence/install.json)/[async](async/a/evidence/install.json)
installer records identify this archive. The installed controller is
`01f38d32f34994f89bea3c75f57eecc506db6157309e74f3fe5425f64775601e`;
no binary/controller replacement follows these installs.

Node `d28d17faa8bb71eb8c0a28241e8556396e440b204cf0242d2af8133700f6a494`
and Meta `64d85fc3933637ccd7fdf4eafded5e5c8cb2354e73b4c8d1a27b2b8d43f4c7e7`
match the [previous RN2/three-mode cohort](../20261001-round1-mainline/README.md).
That cohort retains its original archive/runtime identity and explicit controller
replacement. Its synchronous RN2, Auto/required RXE, gRPC and stop/restart results
are reused; **they are not a fresh corrected-archive RN2/three-mode run**.
[Linux input comparison](reuse-inputs.json) binds all143 unchanged compiler inputs,
controller and protected documents to the already completed
[source gate](../20261001-round1-mainline/gate/full/gate/lib.log):
414 library, 65 contracts, 4 shared errors, 9 local API, 5 privileged FUSE,
fmt, strict Clippy, five features and build. No Rust gate is repeated for these
probe/package/document-only changes.

## Normal user flows

| Flow | Independent proof |
| --- | --- |
| Fresh install and process deployment | ctl/A/B/C preflights, install records, installed binary hashes, central memory Meta and independent OwnerFs/DFS mounts; actual LAN readiness uses the installed controller |
| Workspace/Home and Owner local/remote | [Home A](r1/a/evidence/workspace.json), 4 MiB +17 writes/first reads, [Home physical files](r1/a/evidence/home-physical.txt), B restart/reopen |
| R1 local write | Before starting B/C, both files sync successfully; [all Peer payload counters are zero](r1/a/evidence/metrics-only-local.txt), [Meta confirms one local durable copy](r1/a/evidence/r1-replicas.json) |
| Cross-node file semantics | [10-step report](consistency/report.json): same-mount visibility, close-to-open, remote Owner/DFS writes, resize/provider handles, owner handover and hardlink/rename; actual A/B Node and separate ctl Meta identities checked before/after |
| Healthy async replication | N2/M1 write/sync leaves [one durable copy and Pending debt](async/a/evidence/replication-before-join.json). Nodes join and [both tasks complete](async/a/evidence/repair-completed.json) with two available durable copies |
| Physical and transfer proof | A/C [physical chunks](async/c/evidence/physical-chunks.json) match; B has no durable Chunk and its first read is exact. B/C first reads match bytes/length/EOF. [C counters](async/c/evidence/metrics-final.txt) record real RDMA replica bytes; DFS gRPC payload is zero |
| Normal stop/restart | R1 B restart reopens exact Owner/DFS content; R1 all Nodes and Meta stop with exit0 and own mounts/listeners absent. Async cohort remains running as the next fault-round baseline, not a long-stability claim |

The locked ARM64 Linux ctl2CPU/4GiB, A/B/C2CPU/6GiB, guest ext4 and RXE topology
is unchanged. Build is stopped. [Current live Meta identity](async/ctl/evidence/process-identity.json)
and each Node identity identify the async baseline. Tested data stays on guest
ext4; only an inactive temporary installer extraction on ctl was
[relocated with exact metadata/content comparison](r1/ctl/evidence/extraction-relocation.json)
to preserve preparation reserve. Installed binaries/state/test data were unchanged.

## Validation and retained setup errors

[Audit](audit.json) passes 115 semantic checks on the retained normal-flow records; [negative regression](audit-regression.json) passes eight checks and
rejects wrong archive/policy/content, missing async debt/completion, false suite
PASS and wrong Meta identity. [Linux probe selftest](r1/ctl/evidence/consistency-selftest.json)
checks central Meta qualification and rejects absent/wrong identity and non-AFS
reference mounts.

Earlier [duplicate-worker](consistency-preparation-1/report.json) and
[missing-identity](consistency-preparation-2/report.json) setup failures remain
separate. The final report uses one worker prefix and actual independent Meta
identity; failed attempts are not PASS. A nonroot probe initially hit the mount's
root-owned workspace permission; product probes use root as specified. An initial
physical-copy command assumed B was chosen: actual placement selected C, so
final physical checks derive Nodes from receipts, rather than assume a target.
These setup corrections do not weaken a product assertion.

Major faults, physical ENOSPC/network interruptions, RDMA exceptional teardown,
resource/performance qualification, etcd/Redis, complete POSIX/install matrices,
8 GiB and soak remain in the [issue ledger](../../issues.md). Memory Meta normal
stop/restart does not prove persisted namespace recovery. AGENTS and handoff
remain unchanged.
