# Current 6d51aeb default-OFF installation/recovery — bounded PASS

2026-10-07, existing ARM64 Linux `afs-g1-clean`, guest ext4, no Rust/C compiler.
Same Rust6d51aeb/map66dbbe3e/157 compiler inputs and existing14,728,435-byte release
package used in the managed workspace diagnostics; no build/product/tool changes.
G1 historical8/8 remains complete. This adds the current OFF G2.08/G2.27 delivery
branch receipt; it does not complete G2.27 performance or native ON gates.

| Current bounded result | Evidence |
| --- | --- |
| Necessary dependencies, compiler-free, package/source/ELFs, capacity, ports and new root | [Preflight](results/preflight.json);1GiB minimum, actual5,999,067,136B free; no environment installation/repair |
| Ordinary OFF installation and executing installed inode/path | [35 checks](results/result.json), [before identity](results/recovery-before.json); generated local-file Meta/gRPC/R1, experiment disabled |
| OwnerFs/DFS each64MiB full write/basic checks | [Owner](results/ownerfs-write.json), [DFS](results/dfs-write.json); fsync/close/EOF/full SHA; small append/truncate/rename/unlink/permission/lock/mmap, not POSIX certification |
| Meta-only ordered recovery/full content | [After identity](results/recovery-after.json), [Owner read](results/ownerfs-read.json), [DFS read](results/dfs-read.json); Meta245512→246147 with newer starttick, unchanged ELF, Node245702 and exact mount IDs55/104 unchanged |
| Normal exit0 and mounts/processes absent | [Result/35PASS](results/result.json), [actual commands/status](results/commands.json), [independent postcheck](postcheck.json) |

[Exact outer command](driver.command.json), [exit0/5.155s](driver.exit.json),
[raw stdout](driver.stdout), [raw stderr](driver.stderr), [source map reference](source-map-reference.json),
[actual manifest](results/package-manifest.json), [package file hashes](package-inventory.json),
[unchanged maintained runner](../../acceptance/installed-smoke-linux.py), [predeclared slice](../../installed-off-6d-slice.md).
The same7 [Linux tool guards](../20261007-installed-off/package-build/unit-final.stderr)
are reused with exact tool SHA/predicates in [tool reference](tool-reference.json);
no redundant guard or standard/performance retest. Runtime regression is fresh
because product binary identity differs from the historical25a8061 package.
Bounded independent review is recorded in `final-review.json`.

## Package identity and trial limits

Version `0.1.0-g2-procfree-6d51aeb`, source
`6d51aeb45c1ed8669d80f612b3817e6d1bdabe04`:

- Package SHA256 `ee25d5892c4e884e67c86d4e5b9c6ab551af4c46649edab6a05da06659d1aff9`.
- Meta SHA256 `2c7b7d088b759e3b9375080002182aa484a424b4fa216da1fb79a1004e96168e`.
- Node SHA256 `2cf1f538fe7af332a711f3c66a074ace140c00773826a182709a6445b8ae2645`.

Archive is retained outside Git under the original workspace
`evidence/afs-delivery/managed-workspace-20261007-r1/afs-0.1.0-g2-procfree-6d51aeb-linux-aarch64.tar.gz`.
It contains ordinary installation/selfcheck/config/lifecycle tools and no
experimental container helper/controller or private keys. The packaged guide
records the earlier25a8061 regression, under its original identity; use this
receipt and the current [trial guide](../../../docs/guides/trial.md) for6d51aeb.
No package bytes/version were silently changed to update that historical text.
No package reproducibility rebuild is claimed by this installation-only run.

Ordinary Owner read/write performance failures and container diagnostic data
remain under their own identities. Mixed native append offsets/locks/watch FAIL
remain; bind stays OFF. Official fuser migration R2 is independently blocked by
public API gaps, so this is not proof of an unpatched upstream dependency.
No full standard, crash, multi-node recovery, large/long reliability or all-G2
claim. The earlier [25a8061 OFF receipt](../20261007-installed-off/README.md) and
[append r8 diagnostic](../20261007-append-diagnostic/README.md) remain immutable.
