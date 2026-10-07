# Active workspace Node shutdown: one runtime subcase

2026-10-07; main d82cc7d compiler inputs and fresh release ELFs already verified.

Run the existing official runc in afs-g2-micro with those exact packaged ELFs. Add one mutually exclusive maintained-driver mode: prove a live FinalVerified workspace and read the seed through the container; stop Node while it still owns that workspace, without public workspace Stop; then stop Meta and verify actual wait0, service/supervisor/container PID absence, normal FUSE/control closure and unchanged trusted inputs. Do not run unchanged standards, timings, recovery restart or the 64MiB payload. Keep default OFF and G1 historical8/8; this does not close full G2.12/13.

Before editing, existing coverage includes final mount/source/namespace guards and negative actual-wait receipt tests. Add a targeted negative guard that rejects an Idle workspace before Node stop, and a call-order guard preventing public Stop in this selected path. Tests run only Linux. Package twice from already built ELFs, bind compiler/tool maps and package SHA; admit dependencies, package libraries, rootfs, official runc, ports, mounts, protected processes and capacity before startup. Keep runtime rootfs, ELFs and TLS outside Git, archive only command/results/versions/checksums/index. Real environment blockers stop the affected lane; no repair/rebuild loop. Main direct Lore commit/push; protected handoff/vendor untouched.

## Bounded result

The single actual case passed43 driver/19 independent checks, with direct exact Node/Meta wait0, container/four service-supervisor PIDs absent and normal mount/control closure. Linux21 affected tool guards passed; failures retained. [Evidence and limits](evidence/20261007-ownerfs-bind-active-node-stop/README.md). Next remains workspace core performance; historical diagnostic timings reused and full host-switch/general revocation/semantics open. No product or dependency edits.
