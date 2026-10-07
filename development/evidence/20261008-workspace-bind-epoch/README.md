# Workspace bind: real registration-epoch failure closure

**Fact, 2026-10-08:** a narrow G2.12 boundary was observed with the fixed **f03dc2b3** product, not a new runtime version. Both original driver results remain **FAIL**. An independent audit of the saved r2 evidence passes **22 checks** for heartbeat-driven authority failure, normal owned mount closure and data preservation. Full bind qualification remains open; G1 historical 8/8 and G2 totals are unchanged.

## Identity and observed result

| Input/result | Exact identity or scope |
| --- | --- |
| Source context | [efc1138b](https://github.com/lelezi257/dms/commit/efc1138b67e729e9a44df74093b26aced6a87479); only acceptance tools/evidence/documents changed in this slice |
| Runtime product | [f03dc2b3](https://github.com/lelezi257/dms/commit/f03dc2b3679c31daa51caee275fb2087413e949c), 157 compiler inputs/map `2b17fad77c87b4977d79e14809b6eada755f648767dd29b5503e49556f8ce7b4` |
| Package / ELFs | package `7bfc6b520e551d2281790862c2968e990e376c5c490b2135f583f4da0ccc1972`; Meta `c7447bcfac7e3f8bf605446ade5e11be74f1333709a8f7bb5378b1d6ee7506fd`; Node `3b1f1dce187a6285814c03b9024cdfc5f2dec990a73cba9cc13b3dc9ef402d36` |
| Environment | existing afs-g2-micro, Linux ARM64, guest ext4, local-file Meta, gRPC/mTLS, host mount namespace; no VM/third-party/Rust modification |
| Physical bind | actual `state/node/ownerfs/root-776f726b7370616365-e1` → `mount/ownerfs/workspace`; matching device/inode, different mount ID from FUSE, ext4/nodev/nosuid. [Observed identity](r2--results--binding-before.json) |
| Public trigger | second real DFS Node with same trusted node id/certificate, fresh session, separate data/FUSE/ports, both bind switches OFF; Node health shows epoch **2→3**. No original-node signal during observation |
| Original outcome | actual wait **1**, structured PermissionDenied registration-epoch error, Node and supervisor gone; bind and FUSE absent at **9.938641209999332s**, inside the frozen 35s observation budget |
| Preservation | exact 4096B proof unchanged, replacement actual wait0, Meta separately stopped with exact wait0; protected ELFs and all old mounts unchanged; no remaining owned product processes |
| Scope | session replacement → heartbeat error → Services shutdown → normal bind/FUSE closure; no outstanding native FD/mmap references |

[Independent saved-evidence audit](post-audit.json), [original exit](r2--results--original-authority-exit.json), [replacement exit](r2--results--replacement-actual-wait.json), [173 observations](r2--results--closure-observations.json), [actual error log](r2--final-logs--node.log), [contract](r2--results--contract.json).

## Failures remain failures

- **r1 FAIL:** a second OwnerFs with an empty data directory was safely rejected: `Meta has an active Home root without local catalog evidence`. Its public registration was not independently captured; the original was signalled during failure cleanup. This is not natural-shutdown evidence. [Result](r1--results--result.json), [commands](r1--results--commands.json), [rejection](r1--results--replacement-logs--node.log).
- **r2 driver FAIL:** the natural closure and actual error exit were recorded, but the error-string assertion omitted `the`. Cleanup then used `stop all`, which stopped at the already-failed Node receipt and left Meta running. [Original result](r2--results--result.json), [commands](r2--results--commands.json). The owned Meta was then stopped separately: [stdout](r2--results--post-meta-stop.stdout), [actual exit0](r2--results--post-meta-stop.exit). The independent audit verifies its exact incarnation/wait0; it does not rewrite the original result.
- Maintained [driver](../../acceptance/workspace-bind-epoch-linux.py) now uses the DFS trigger, structured exact authority error, and independent per-role failure cleanup. [Eight targeted Linux guards](../../acceptance/test_workspace_bind_epoch.py) pass, including preservation of the failure when Node cleanup fails while Meta still closes. **The corrected final driver was not rerun as a product case.**
- Initial audit orchestration had an incorrect host working directory; no audit or product process ran. Corrected read-only audit passed. No environment repair or third product attempt occurred.

## Provenance, restoration and remaining boundary

[Initial plan](plan.md) and [pre-r2 amendment](r2-contract-amendment.md) preserve the fixture change. Exact [r1](tool-inputs.json), [r2](tool-inputs-r2.json) and [maintained](maintained-tool-inputs.json) maps are separate. Instead of copying full tool snapshots into Git, [r1 delta](r1-to-maintained.patch) and [r2 delta](r2-to-maintained.patch) restore both executed versions from the maintained entry with `patch -R -p1`; Linux restoration checked all **16** mapped tool files.

[Source impact](source-impact.json): all 157 compiler inputs are unchanged from the efc1138b source context. The sole existing difference from the fixed f03 source is the previously qualified test-only native_home_fuse_fixture.rs; it is not a change in this slice.

Complete nonsecret raw archive stays outside source at `evidence/afs-delivery/workspace-bind-epoch-20261008-r1/guest-evidence.tar.gz`: **47,720 bytes**, SHA256 **9343e4babcd0c046a074451647a4060d1ba0e014ba7a96df46da835a55f7ab1e**. [Raw index](raw-evidence-index.json) covers **248 files**, including both command streams, failures, receipts, final logs, retained state/data and the one-shot audit command. Actual Linux extraction verified every SHA. TLS private keys, ELFs, VM disks and full tool copies are excluded. Nothing was deleted from either guest case root.

**Fact:** production Node heartbeats detect the registration epoch change (`src/node.rs:1349`) and Services stops the bind worker; Node joins that worker before FUSE (`src/node.rs:1788`). The bind worker's 100ms check consults cached RootManager authority. Runtime code does not call `RootManager::on_watch_disconnected` / `invalidate_all`; Meta root-command watch has no production Node subscriber. **This case does not prove immediate authority revocation, runtime root-command invalidation, durable revoke/ACK, denial through existing native FDs/mmap, full ON POSIX behavior or performance.** Those remain explicit gaps, not waived gates. Next independent entry remains the workspace bind function ledger; ordinary remote-read comparison remains observer-blocked, with saved timing data retained.
