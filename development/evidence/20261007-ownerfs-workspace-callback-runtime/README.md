# Actual callback witness: ordinary FUSE and managed native paths

2026-10-07, product mainb31292ac56dc3b88ec0f0dca09cbce014af1e7fb/map58a71572; [source/build/tests](../20261007-ownerfs-workspace-callback-source/README.md). Existing ARM64 Linux afs-g2-micro/ext4/official runc1.5.2 reused. New Meta SHA75864631b10ec053bb3ed09cc00dbf57befc967413be721cd539ec976d3bebb2 and Node SHAd54c2dc27f7915aaf907f7741699f91508d2e2c147dc9445de3ed72f08279ea9 actually deployed in a fresh fixture; previous1451f60 install untouched. [Two byte-identical packages](package-source-proof.json), package SHA3d1e68de2a57cd15f0c942fa3ee32662c283175cc0bff1f33f6e8a3cf810d390. No full timing/standard/64MiB/recovery rerun.

[One-shot complete admission](outer-admission.json) before install/start qualified dependencies, actual package ARM64 ELF libraries, six rootfs inputs/runtime SHA/ldd, ports, RAM, mounts/protected processes and budget. [Actual invocation](driver-command.json), [selected result](results-r1/result.json): 49driver checks PASS. [Independent postcheck](outer-postcheck.json): 27checks PASS, rereads raw scrape outputs and direct lifecycle receipts instead of trusting the summary alone.

## Selected observed result

| Payload window | OwnerFs read callback delta | OwnerFs write callback delta |
| --- | --- | --- |
| Ordinary FUSE path positive control | 2 | 1 |
| Managed native path | 0 | 0 |

The ordinary FUSE witness writes/fsyncs/closes67,584B, then hints POSIX_FADV_DONTNEED and reads the exact data. The hint is not cache qualification; actual positive callback deltas prove callbacks occurred. The native Exec copies the32KiB-class host sentinel, syncs the destination and independently checks its full SHA in the container. Actual executed runc argv/output is retained under control. Both paths use one experimental-ON Node and actual Home identity, not two OFF/ON configuration timing cohorts. The native window excludes setup and cleanup and makes only read/write bypass claims. All34 callback deltas are retained, with zero samples explicitly observed. No missing series or RPC metrics are substituted for zero.

[Raw before-FUSE scrape mapping](results-r1/callback-before-fuse.json), [after-FUSE](results-r1/callback-after-fuse.json), [before-native](results-r1/callback-before-native.json), [after-native](results-r1/callback-after-native.json) point to original raw stdout and exact curl argv in [commands](results-r1/commands.json). [Selected deltas/incarnation](results-r1/callback-result.json); [actual mount/source/namespace/unique ID](results-r1/final-identity.json). Live Node ELF/starttick remains unchanged. These are implemented first-party callback entries, not all kernel wire opcodes, syscalls, per-file counts, throughput or cold/hot proof.

## Closure and remaining scope

[Actual wait0 receipts](results-r1/orderly-final-actual-waits.json) bind exact Meta/Node config/executable/boot/starttick and supervisors. All four service/supervisor PIDs and the actual container are gone; runc list empty, FUSE/control socket/lock closed normally. Independent observations preserve previous installed ELFs/protected identities and satisfy256MiB/1GiB capacity limits. Runtime-mutated rootfs/binaries/invocation glue remain outside Git at [checksummed locations](local-only-index.json), with the [excluded runtime-root inventory](results-r1/orderly-final-runtime-roots-local-only.json). No source/Python/ELF snapshots added here.

This completes only callback observability and a short data-path witness. Historical [6d timings](../20261007-container-perf/README.md), [d82 active Node shutdown](../20261007-ownerfs-bind-active-node-stop/README.md), standard/recovery results and failures retain their versions. G1historical8/8/G2counts/defaultOFF unchanged. Full G2.12/13, host-visible switch, general revocation/reconciliation and mixed append/classic locks/watch remain open. Follow the agreed core case order; next is a short create/delete callback witness (a missing counter case, not unchanged timing retuning). Ordinary performance tuning, cache/large/long and complex reliability remain later. [Narrow plan](../../ownerfs-workspace-callback-counts.md). Main direct Lore push; remote ref/tree verification lives with raw evidence outside source after publication.
