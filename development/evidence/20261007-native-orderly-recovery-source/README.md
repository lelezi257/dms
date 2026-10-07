# Final orderly-recovery draft: affected Linux release checks

2026-10-07. After user-authorized archival relocation restored37.64GiB, run only the affected source gates. **Four selected gates PASS; not a complete source gate, build, new package or runtime recovery PASS.**

Exact Rust `src/node/native_workspace.rs` SHA8c6f1d5021aed4280a4c05d8bf371f624b4a8f31d9c847f4b98b1a5a2971556b, gate7df1cca3dd7121d5ff3835dd9a18145a3a7140180a8a80ae2d1a3734be0dab20,157 input map e15c7eab07d0a58376ce0fe260526eefa8bbb3e3185d95185211c2bdaa0273be. Existing [frozen input manifest](../20261007-native-orderly-recovery/compiler-inputs.json), checked at entry and exit; exact compiler files unchanged. Documentation commits do not change this map.

[Preflight](preflight.json) PASS: ARM64 Linux/ext4, Rust/Cargo1.95, dependencies/sudo/FUSE,240 offline packages, capacity≥3GiB/no profile overrides/no concurrent Cargo. Ordinary-user Cargo uses the existing release target; only the actual compiled test binary runs as root. No root Cargo, debug build, installation, resize or current-cache deletion.

[Source proof](source-proof.json) records fmt, native-control, physical-build, physical-native all exit0. [Ordinary tests](native-control.log):10PASS/7 explicitly ignored privileged tests. [Physical root tests](physical-native.log):7PASS/0ignored, including three rootfs trust/reuse/OCI workspace fixtures. [Exact test ELF](test-binary-identity.json):27,532,424B/SHA884ba8f9cb3a6949354d4262542469bb626c890c11c427ab3733ea33bc06bb3c. No ELF copy is stored in this packet. Final test result is bound to the final source; earlier pre-addendum10+4 remains historical with its missing-source-SHA gap.

The source-proof `binaries` dictionary is incidental existing release binaries, **still old6d** because `build` was not selected. It does not establish that the new draft has runnable service binaries. No full library/contracts, Clippy, release bins, package or actual restart was selected in this run. Next: necessary remaining source checks, then a distinct new package and one4KiB orderly recovery. Standard suites and old performance data remain reused at their documented scope; G1/G2 counts and defaultOFF unchanged.

A follow-up identity reader initially chose the timeout argument180 as a filename; [tool read error](identity-read-first-attempt.json) is preserved. Corrected read selects the actual absolute release test-binary argument; no build/test repeated.
