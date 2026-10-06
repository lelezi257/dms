# Current default-OFF installed regression

**PASS in bounded G2.08 scope:** source `25a8061a82b7a8629c9a06486d712d32507f2ee7`, compiler map `8ef8b7882381b80445ce13576a6b6c41b1a054fd5a101a5143d77b32c14ea267`. No Rust/product behavior changed in this slice. G1 remains historical8/8; G2.27 performance-package exit and G2.12/13 actual container gates remain open.

| Result | Evidence | Scope |
| --- | --- | --- |
| Reproducible Linux package PASS | [package receipt](package-build/package-proof.json), [tool hashes](package-build/tools.json) | Two same-input builds of existing qualified release ELFs produce identical14,727,158-byte archives. Commands transcribed from orchestration; stdout/stderr retained. No new Cargo build |
| Compiler-free install PASS | [admission](r2/preflight.json), [manifest](r2/package-manifest.json), [commands](r2/commands.json) | Existing afs-g1-clean ARM64 Linux/ext4, no cargo/rustc/C compiler, fresh isolated install. Git exists on VM but is neither required nor invoked; not a Git-free VM |
| OwnerFs/DFS basic integrity PASS | [Owner write](r2/ownerfs-write.json), [DFS write](r2/dfs-write.json) | Each64MiB, full pattern/SHA/EOF, fsync/close/reopen, bounded small append/truncate/rename/unlink/chmod/fcntl/mmap. Not pjdfstest/full POSIX |
| Orderly local-file Meta recovery PASS | [before](r2/recovery-before.json), [after](r2/recovery-after.json), [Owner read](r2/ownerfs-read.json), [DFS read](r2/dfs-read.json) | Parent directory fsync, only Meta restarted; new Meta PID/starttick, unchanged installed ELF/inode, Node identity and exact mount IDs55/104. Full content SHA/size matches. Not crash or multi-node recovery |
| Normal cleanup PASS | [result](r2/result.json), [managed records](r2/process-records/meta.identity), [commands](r2/commands.json) | Both managed exits0, original before/after processes absent, exact mounts removed. findmnt exit1/empty stdout is expected absence |
| Evidence-guard regressions PASS | [7 Linux tests](package-build/unit-final.stderr), [exit0](package-build/unit-final.exit) | Reject wrong executable despite same bytes/hardlink, Node/mount changes, invalid Meta restart, either backend corruption/short read/failed probe |

Package SHA256 `77cbae8914715cd08d6ba3771e0219111f18d5cc0475f42661e4913873bfb18b`.
Meta SHA256 `08aee9fec2e7e12ce03912fa85e8bded28b2162de78c2c9c36703a9a1a6e58b8`.
Node SHA256 `d6e7cd11a610e7f0839d2a3d2337cd92678432e15fcffb6fd2fa6a00860058b8`.

`tools.json` records the frozen transferred build payload, not a claim that every entry equals a later checkout. The packaged trial guide matches main25a8061; this publication's guide adds the result link afterward. AppleDouble/pycache entries in that inventory are transfer artifacts, excluded from the archive and not required/executed package inputs. Maintained final runner/tests and actual deployment scripts match their recorded hashes.

## Review and retained first run

[r1](r1/result.json) passed its narrower same-ELF-hash/content/restart/cleanup checks. Independent review found this did not prove the executable came from the fresh installed prefix. The maintained runner now checks device/inode and resolved path, records mount IDs and verifies both process incarnations after stop. [r2](r2/result.json) reruns only this affected installed regression with the same package/ELFs on a fresh isolated root. No standard suite or benchmark was repeated. [Final review](final-review.json).

No Python source snapshots here. [r1→final delta](driver-r1-to-final.patch) reversed against the maintained runner reconstructs old runner SHA `6986dc856cc3b9441b0aa8ea5f0df543ce53be66cd018130ed37cb4469e12ec5`, verified on Linux in the package receipt. Final runner SHA `2e857135d05e5e7380d3dc5f4cec51a80a7045541c7f5f3fc708c6dffc9c1bea`. [Maintained runner](../../acceptance/installed-smoke-linux.py), [targeted tests](../../acceptance/test_installed_smoke.py), [slice/reproduction](../../installed-off-slice.md).

## Delivery limits

Version `0.1.0-g2-off-25a8061`, all-feature release ELFs, native **OFF** by generated default, local-file Meta/gRPC/R1. Archive contains ordinary trial tools, excludes experimental workspace helper/controller; actual ON is unsupported. Container admission remains [BLOCKED by missing runc](../20261007-native-workspace/runtime-admission-observations.json), environment question pending.

Archive stays outside Git in original workspace `outputs/releases/afs-g2-off-25a8061/`; transfer separately and verify SHA. This limited trial regression is not a qualified performance release. Old pjdfstest/performance results retain original identities; ordinary FAIL/data unchanged. Packet contains text receipts/raw logs/delta only, no ELF, full scripts or private TLS keys.
