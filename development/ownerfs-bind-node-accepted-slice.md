# Current Node accepted workspace startup: one bounded regression

2026-10-07, main1451f6066c09d9761404defa8852f13f8b0c0d2f. This continues G2.12 after the rejected-startup fix; G1 historical8/8 stays closed and full ON/G2.13 stay open.

## Why this case is needed

The Node success registration moved into register_native_workspace_startup in73842cd. Its two new unit tests and actual process test prove rejected startup; historical3cc orderly recovery and6d managed lifecycle exercised the older success wiring. The extracted OwnerFs bind core has passed a genuine FUSE component test, but that test does not start the Node/runc adapter. Supplement only the current success branch and normal cleanup; do not repeat standards, append/locks/watch diagnoses or timings.

## Frozen scope and execution

1. Reuse existing Linux build proof:157-map196a4177141fb837d3e5b155438e894ab9d632bfceb97a9ea3ab20bf2ce57c6d, Meta f4d423fcced38cd0a6624b1104ff05c136974150d21590522a90a40e637aca0c, Node de1d16895c062651aa9e9de375b9ac5bb5da5b52672a7d70093bebdd090bc9ee. Package already-built binaries twice on afs-build with fixed main commit/version/epoch; require exact package byte identity and manifest/binary hashes. No Cargo rebuild, new dependency, source snapshot or ELF in Git.
2. Reuse isolated afs-g2-micro, official runc1.5.2 SHA d10ecae898361832a059be2089bab92d158aec54661b18ed7346ed79628b46b0 and six pinned regular rootfs files/helper c9c9fd1a…. Preflight the complete selected dependencies/tools/rootfs/package/ELFs, ports24400/24401/24500/24501, FUSE, ext4,512MiB memory margin,256MiB allocated fixture/transport budget and1GiB free floor. Record existing process/mount identities before installation; actual environment mismatch stops this lane and requests help.
3. Use unchanged maintained native-workspace-linux.py with semantics-only and only permissions_errno, plus unchanged native_mixed.py and canonical control CLI. One fresh /opt/afs-main-bind-accepted-20261007-r1 root, separate /var/tmp/afs-main-bind-accepted-20261007-r1 transport/results. Skip64MiB payload, performance, restart and known failing groups. Observe actual installed/live Meta/Node, FinalVerified source/namespace/unique mount/flags; selected tiny permission/error cases must pass on native reference and mixed FUSE/native views. Read/write denials for uid501 against root0600, ENOENT/EEXIST and unchanged secret/mode must be retained with actual command receipts. This is a regression, not new POSIX coverage.
4. Observe actual public Stopped→Idle, empty runc list, ordinary final-clone/export detach, actual Node/Meta lifecycle receipt wait0, service/supervisor/container PIDs gone, no owned mount/socket/lock, and unchanged protected processes/mounts. Preserve all output and any FAIL. No force/lazy recovery, environment repair or rerun merely for noise.
5. Keep package/TLS/rootfs outside Git. Publish only commands, version/hash maps, results and evidence index; direct Lore commit/normal main push. Overall project review occurs after the overall goal, without a feature approval gate.

## Boundaries

This is the existing runc adapter in its private mount namespace, not the missing standalone host-visible OwnerFs switch. Physical Home→matching first-level FUSE covering bind and container final view must be verified, not inferred from names. Direct ordinary-caller admission, stale/revoked native-FD fencing and EBUSY ownership across Node failure require additional implementation; periodic permit checking alone is insufficient. Known append/classic-lock/watch failures and production READY/general drain remain open. Current package regression does not automatically make it the recommended trial or qualify performance parity.

## Bounded closure

Executed once on existing afs-g2-micro: selected permissions_errno native-reference and mixed paths PASS;50 driver checks,30 independent postchecks, actual Node/Meta wait0 and protected-state/capacity proof PASS. Two prebuilt-ELF packages are byte-identical. [Exact current version/results](evidence/20261007-ownerfs-bind-node-accepted/README.md). Standalone feature/full ON/performance remain open; source and acceptance tools were unchanged.
