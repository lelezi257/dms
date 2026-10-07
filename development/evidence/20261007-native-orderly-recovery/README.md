# Container orderly recovery: draft repair, environment blocked

2026-10-07. **No new product PASS.** Published main f09185e and installed product6d51aeb/map66dbbe3e/157 inputs retain their original scope. This branch holds an unvalidated first-party repair; no new ELF/package or two-phase runtime exists. G1 stays8/8, G2 stays10 limited complete/2 performance FAIL/2 bind in progress/13 pending. Bind default OFF; 3FS qualification deferred by user.

| Item | Evidence / exact scope | Status |
| --- | --- | --- |
| Restart incompatibilities | [6d code and stopped fixture metadata](rootfs-restart-static-boundary.json): runc device/link entries fail strict template validation; reset sequence collides with retained command records | Static facts; actual second startup NOT RUN |
| Narrow repair | [Plan](../../native-rootfs-restart-repair.md), [addendum](../../native-rootfs-restart-records-addendum.md), [157 draft inputs](compiler-inputs.json); only native_workspace.rs compiler delta | DRAFT, not validated candidate |
| Tool regression | [8+1+1](tools-preparation-r3.json), [shared archive +1](tools-preparation-r4.json) Linux guards, unchanged results reused |11 tool PASS; no product PASS |
| Preliminary Rust output | [10 tests](build/cargo-test-native-workspace-2.txt), [4 privileged tests](build/cargo-test-native-mount-ignored.txt), before sequence addendum | Exact tested source SHA missing; cannot qualify final candidate |
| Independent review | [Combined review](independent-final-source-tools-review-r1.json), [default-entry FAIL](independent-default-test-entry-review-r1.json), [minimal fix review](independent-default-test-entry-review-r2.json) | Static approved within trusted-admin/no-concurrent-mutator boundary |
| Build VM | [ENOSPC](build/enospc-after-clippy.txt), [command provenance](build/provenance-native-rootfs-repair.json), [root guest recheck](build/root-postblock-guest-disk-process.txt) | BLOCKED:85GiB root full, no Cargo/rustup/rustc listed by guest check |
| Final sequence / source gate / package / recovery | [Sequence attempt](build/cargo-test-command-sequence.txt) failed before tests; final3 rootfs tests explicitly ignored by ordinary library and included with old4 in sudo physical gate | PENDING; no product admission/runtime |

Executor mistakenly used debug/test artifacts and sudo Cargo triggered root-side toolchain installation instead of planned warm release cache. Another filtered test after ENOSPC also failed before execution; this violated the stop-on-environment-block rule. Branch now stopped; no cache deletion, disk expansion, ownership repair or environment rebuild. User assistance requested.

Final Rust SHA:8c6f1d5021aed4280a4c05d8bf371f624b4a8f31d9c847f4b98b1a5a2971556b. Gate SHA:7df1cca3dd7121d5ff3835dd9a18145a3a7140180a8a80ae2d1a3734be0dab20. Draft157-input map:e15c7eab07d0a58376ce0fe260526eefa8bbb3e3185d95185211c2bdaa0273be. Final-hash format/tests/strict Clippy/build pending. Preliminary10+4 never transfer.

[Inventory provenance](inventory-command-provenance.json) reconstructs argv/script from tool-call history; original stdout raw bytes missing. [Build provenance](build/provenance-native-rootfs-repair.json) records missing pre-addendum source SHA. Gaps are explicit; no invented identity.

[Selection index](text-selection-index.json) maps original bytes; full Rust snapshot remains local. [Durable local archive index](root-local-only-input-index.json), tool maps and small reversible deltas preserve failed/predecessor tools; tar archives stay outside Git. No historical conclusion, vendor or handoff changes.

Next after environment resolution: one identity/profile/ownership/capacity admission, release/offline/locked build as existing user, sudo only exact built physical-test binary; new ELF/package then one4KiB two-phase normal recovery. No standard/64MiB/performance/3FS repeats or abnormal adoption matrix.
