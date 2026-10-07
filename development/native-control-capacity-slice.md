# Active workspace: bounded operation ledger exhaustion

2026-10-07. Independent G2.12 runtime subitem, following the user's return from deferred 3FS qualification to container workspace. G1 historical8/8, full G2.12/G2.13 and all published failures remain unchanged. Bind stays default OFF.

## Frozen inputs and admission

Reuse product6d51aeb45c1ed8669d80f612b3817e6d1bdabe04, the157 compiler inputs/map66dbbe3e0071fcec1efc9cf370c709a99f025d39acd4f37ba57582c874429304, packageee25d5892c4e884e67c86d4e5b9c6ab551af4c46649edab6a05da06659d1aff9, Meta2c7b7d088b759e3b9375080002182aa484a424b4fa216da1fb79a1004e96168e, Node2cf1f538fe7af332a711f3c66a074ace140c00773826a182709a6445b8ae2645 and official runc1.5.2/d10ecae898361832a059be2089bab92d158aec54661b18ed7346ed79628b46b0. No Rust/vendor build or product/package change.

Use existing ARM64 Linux afs-g2-micro, fresh root `/opt/afs-managed-control-capacity-20261007-r1`, guest tools/raw `/var/tmp/afs-managed-control-capacity-20261007-r1` and research evidence `evidence/afs-delivery/native-control-capacity-20261007-r1`. Once before execution verify dependencies, root/sudo, suite/tool hashes, six pinned rootfs inputs, runtime/ELF/ldd/package identity, fresh paths/ports24400/24401/24500/24501, ext4/RAM/disk and protected existing process/mount inventory. New root plus guest tools/raw allocation ceiling256MiB; final free-space floor1GiB. No installation, resizing, rebuilding or environment repair. A genuine blocker stops this affected item with retained evidence and a request for help.

## Exact selected case

Existing in-process capacity test covers ordering but not live owned-container cleanup. New runtime coverage must prove:

1. One successful public Start establishes FinalVerified. It is the only pre-probe cached operation. Skip old malformed/absent/replay/busy/source-injection checks,64MiB payload and semantic/performance groups. Snapshot grant, container ID, actual PID/starttick/boot, namespace, exact workspace mount/source device+inode and every controller runtime-command artifact name/SHA. Status and external read-only runc state do not consume ledger entries.
2. Send63 distinct otherwise-valid Start requests for the same active workspace. Require63 distinct fresh IDs and exact EBUSY (`Device or resource busy (os error 16)`), statusERROR and production_ready=false. Preserve every request/response. Together with the successful Start these fill the fixed64-operation ledger; duplicate IDs, premature errors or accepted requests fail the case.
3. Send a fresh valid Exec that would write a previously absent sentinel if executed. Require exact ENOSPC (`No space left on device (os error 28)`), statusERROR and production_ready=false. Require the sentinel absent and the full observed active identity/runtime-command artifact map unchanged. A generic error or an executed command subsequently cleaned up is insufficient.
4. Send two distinct Status requests after exhaustion. Both must remain FinalVerified with unchanged identity and no controller runtime-command side effects.
5. Public Stop after exhaustion must succeed; observe Idle, actual container PID absent and empty runtime list. Retain actual Node/Meta processctl closure waits, capacity postcheck and exact protected process/mount audit. No force/lazy unmount or unrelated process cleanup.

Reuse the unchanged lawful20-byte Exec result from [source-rejection evidence](evidence/20261007-native-source-rejection/README.md); no new successful Exec or performance run is selected. The fixed controller's public Stop uses identity-bound `runc kill --all <owned-container> KILL`, verifies Stopped, exact final clone/source/namespace, normal unmount, runtime deletion and export detach. This is existing bounded cleanup, not graceful application shutdown or production drain/revocation ACK. Save actual argv/exits; do not invent an external post-stop namespace observer.

## Verification and stop line

Affected Linux oracle tests must reject wrong busy count, duplicate IDs, wrong/early-full errors, accepted over-capacity Exec, changed observed identity/command artifacts and an existing forbidden sentinel. Reuse unaffected previous guards. Review fixed tools and one-time admission before one live run. Preserve first failures and exact tool versions; no unchanged retry or criterion waiver. Publish text results, commands, versions, SHA/index and fixed-Git source provenance without repeated source snapshots, archives, binaries or secrets.

PASS means only: a filled operation ledger rejects new Exec without active-workspace/runtime side effects and still permits Status and existing owned Stop cleanup. It does not qualify arbitrary restart/failure matrices, full native ON, POSIX, performance, append/lock/watch compatibility or production draining.
