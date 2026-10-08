# G2.15 server frame candidate — Linux binary ready only

**PASS_LINUX_RELEASE_BUILD_ONLY.** No deployment, performance measurement, product acceptance closure or package change. The retained production READ version remains unchanged. This continues the [scoped candidate checks](../20261008-owner-remote-write-server-frames/README.md); their four distinct tests are reused for the identical160-input candidate map, not rerun or counted again.

[Build command](command.json) ran once on existing aarch64 `afs-build`: `cargo build --offline --release --bin afs-node`, exit0. Linux `--help` exited0; no print-config or filesystem runtime check was run. The two existing peer fallback dead_code warnings remain. [Build receipt](build-result.json).

[Candidate identity](candidate.json): source main7589acb2 plus fixed patch854ec68f;160 compiler-input map ec7e2de5. The25,121,416B AArch64 ELF has SHA256 `d14e3182260e0f7ab51c82c4f18b9e2deab6446cd2980a6787fa43c22e9361b4`. It is retained outside Git at the recorded host path. The baseline ELF remains839e14d9. The patch changes only the Owner-only TCP server receive-frame setting; its connection-wide scope and regression limits remain as recorded in the preceding evidence.

[Linux identity verification](verify.json) checks ELF architecture, length and SHA. [Production restoration](production-restore.json) confirms all160 original compiler inputs map to81c30c95 and the product tree is restored. No service, fixture, VM, third-party source or active candidate was changed. Build filesystem free space afterward20,722,229,248B.

[Archive index](archive-index.json):15 build/verification records,6,530B, SHA256 `57439e3ecfeba07ad280e5f0dddf508aea443a483c3500c17ae4cbbefa6867ad`; actual Linux extraction/checksums passed. The archive excludes the ELF. Its retained_binary_inputs field names the unchanged baseline; the new ELF identity is in candidate.json. The candidate patch/input-map recovery remains in the preceding candidate archive; no full source snapshot is added here.

Next: the previously asked write-fixture choice remains pending. After it is resolved, perform fresh peak-space admission and one frozen839 baseline versusd14 candidate pair, reusing an existing64MiB write file. Do not rebuild, rerun the four unchanged tests, waive the preceding FAIL_BUDGET, or treat this build as MooseFS performance qualification. G1 historical8/8, original G2 counts, b80 ON trial and earlier performance/failed records retain their identities.
