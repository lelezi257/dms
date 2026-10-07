# Final orderly-recovery source: remaining gates and candidate package

2026-10-07. **Facts:** candidate source commit3cc10a26b6c5fc4a2edc34f1bf210f4c4849082e; fixed157 compiler-input map e15c7eab07d0a58376ce0fe260526eefa8bbb3e3185d95185211c2bdaa0273be. These are the remaining selected checks after [affected checks](../20261007-native-orderly-recovery-source/README.md), not a repeat of them.

[Gate proof](source-proof.json): library565PASS/19explicit ignores, strict all-target/workspace/all-feature Clippy, release service build and helper-idle all PASS in ARM64 Linux. Inputs checked at entry/exit unchanged. Full library includes the ten ordinary native tests; counts are not added as independent cases. Root physical seven tests are in the earlier packet. No whole contract matrix, standards, performance or runtime restart was selected here.

[Package receipt](package-build-receipt.json) and [13 fixed packaging inputs](packaging-inputs.json): package a5bdc4faab13e3a118a43194d0124946a8e1a05e5ddfb1b2bbba6b9d887355fe /14,731,638B; Meta8c53c693023249b69c22f51d8aad7d0950b0f1d3452534e219c3d1c7c906e286 /15,816,704B; Node738ea5eaf0fe8f6412c99f79e26d1e3a92fba78a068510e7e27b129301d85324 /25,943,584B. Archive SHA agrees at build VM, host and runtime VM. The unchanged probe ELF listed by source proof is not packaged. Package-byte reproducibility machinery is reused from its unchanged historical check; no new Rust-linker reproducibility claim.

[First transfer failure](package-transfer-first-attempt.json) is preserved: ordinary rsync could not create a temp file in the existing root-owned preparation directory. Correct transfer used fresh user-owned staging followed by sudo install; no permission changes or environment repairs. Runtime admission and actual two-phase recovery remain separate evidence. The default-OFF public6d trial remains unchanged until an explicit release decision; G1/G2 counts unchanged.

Large archive/ELF are retained outside Git in project evidence, not copied into this packet. This packet stores commands, output, exact source/ELF/package identities and failed collector/transport record only.
