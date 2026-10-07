# Current Node accepted workspace regression

2026-10-07. **Fact:** one fresh Linux ARM64 run passed on main product source `1451f6066c09d9761404defa8852f13f8b0c0d2f`. This supplements the success registration changed in 73842cd; the previous [startup rejection](../20261007-ownerfs-bind-node-startup/README.md), [3cc orderly recovery](../20261007-native-orderly-recovery-runtime/README.md) and [real FUSE core](../20261007-ownerfs-bind-core-fuse/README.md) retain their own identities. No Rust, maintained acceptance tool, third-party source or environment repair was needed. G1 historical 8/8 stays closed; full G2.12/13 and the stage counts remain open/unchanged.

## Identity and admission

| Input | Exact identity |
| --- | --- |
| Compiler inputs | 157 files, map196a4177141fb837d3e5b155438e894ab9d632bfceb97a9ea3ab20bf2ce57c6d; [original map](../20261007-ownerfs-bind-node-startup/compiler-inputs.json), unchanged Linux build/check proof reused |
| Meta ELF | f4d423fcced38cd0a6624b1104ff05c136974150d21590522a90a40e637aca0c |
| Node ELF | de1d16895c062651aa9e9de375b9ac5bb5da5b52672a7d70093bebdd090bc9ee |
| Package | 0.1.0-g2-main-1451f60, 14,771,720 bytes, SHA745d8e7c4727b89069403b2ca977719bfb1e523e90e9c4a2b7225f6d06340837; two existing-ELF package runs with epoch0 match exactly |
| Official runc | v1.5.2, SHAd10ecae898361832a059be2089bab92d158aec54661b18ed7346ed79628b46b0 |
| Runtime environment | Existing afs-g2-micro; fresh /opt/afs-main-bind-accepted-20261007-r1; transport /var/tmp/afs-main-bind-accepted-20261007-r1 |

[Package/source proof and exact packaging argv](package-source-proof.json), [package manifest](results-r1/package-manifest.json), [seven maintained tool identities](tool-inputs.json), [six regular rootfs inputs](rootfs-inputs.json), [complete preflight](outer-admission.json). Admission verified dependencies, ports, FUSE/ext4, library resolution, exact package ELFs, rootfs, runc, available RAM/cgroup margin and protected live process/mount incarnations before installation. Initial owned allocation15,028,224B, free9,135,652,864B; ceiling256MiB, free floor1GiB.

## New bounded results

- [Exact driver argv](driver-argv.json), [actual exit0, elapsed3.209s](driver-command.json), [result](results-r1/result.json), [50 driver checks](results-r1/checks.json), [raw command receipts](results-r1/commands.json).
- Actual installed/live Meta67202 and Node67386 match the pinned ELF/device/inode/start ticks. [Running identity](results-r1/running-identity.json).
- Physical Home directory `/opt/afs-main-bind-accepted-20261007-r1/state/node/ownerfs/root-776f726b7370616365-e1` exports over the matching first-level workspace in the controller private mount namespace; final container67552 is verified against that physical source, namespace and unique mount identity with nosuid/nodev. [Final identity](results-r1/final-identity.json), [actual OCI configuration](results-r1/control/bundle-afs-native-first/config.json). This does not prove ordinary host-visible bind admission.
- Only permissions_errno selected: root0600 read/write denial for uid501, unchanged secret/mode, ENOENT and EEXIST on the [native reference](results-r1/semantics-reference/result.json) and [mixed FUSE/native views](results-r1/semantics/result.json). Each reports one selected group PASS; this is not a full POSIX suite or new standards count.
- Public Stop→Stopped, Status→Idle, container PID gone, empty private runc list and ordinary detach/cleanup PASS. [Thirty direct postchecks](outer-postcheck.json) independently read actual child/ready/exit receipts: Node and Meta each wait0, all four distinct service/supervisor PIDs gone, owned processes/host mounts/socket/lock gone, protected identities unchanged. Final owned allocation113,041,408B, free9,037,639,680B, within admission budget. Runtime private roots are retained outside Git with their [excluded-root index](results-r1/orderly-final-runtime-roots-local-only.json); no ELF, TLS keys, rootfs copy or repeated maintained script snapshot was added to this packet.

## Reuse and remaining work

Default OFF remains unchanged. No standard-suite rerun, 64MiB payload, timing comparison, service restart, locks, append or cross-watch retest was selected. Their historical successes/failures remain linked from [the acceptance table](../../trial-release-goals.md). Package installation here is for this regression; it does not replace the historical6d public trial or qualify G2.27 delivery.

The accepted current managed lifecycle subcase is now complete. Standalone host-visible switch/lifecycle, native-FD revocation and busy-mount ownership/drain, production READY/restart reconciliation and the known mixed-path semantic gaps remain unfinished. Next follow the existing workspace performance priority with an explicitly limited case; do not wait for or represent full ON acceptance. [Execution boundary](../../ownerfs-bind-node-accepted-slice.md).
