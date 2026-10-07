# Current container workspace: small sequential data case

**Fact, 2026-10-07:** product93169c8 release ELFs, same157 compiler map as the [qualified host source](../20261007-ownerfs-workspace-host-entry/README.md), package reused from [host runtime](../20261007-ownerfs-workspace-host-runtime/README.md). New maintained [driver](../../acceptance/workspace-bind-data-perf-linux.py), [criterion guards](../../acceptance/test_workspace_bind_data_perf.py), [predeclared slice](../../workspace-bind-data-perf.md), [nine exact runtime inputs](tool-inputs.json), [tested inputs](tested-inputs.json). No Rust/vendor/C change or rebuild. Mainbbeebca was the starting Git anchor; this packet is directly committed/pushed on main.

| Independent current case | Paired speed/ext4 median | Five paired range | Outcome |
|---|---:|---:|---|
| G2.13 data: ON sequential write+fsync | 0.944934 | 0.723305–1.181898 | **PASS**, preset>=0.90 |
| G2.13 data: ON sequential read+close | 1.036200 | 0.879088–1.244150 | **PASS**, preset>=0.90 |
| OFF sequential write diagnostic | 0.738207 | 0.583525–0.970480 | **FAIL**, retain; ordinary-path tuning postponed |
| OFF sequential read diagnostic | 0.348338 | 0.267044–0.361045 | **FAIL**, retain; ordinary-path tuning postponed |

These are medians of five **paired reference wall_ns/experiment wall_ns** ratios, not ratios of throughput medians. All pairs/timings/content checks remain in [OFF](runtime/off-analysis.json), [ON](runtime/on-analysis.json), six `*-round-*.json` per cohort and [raw commands](runtime/commands.json). [Absolute throughput supplement](throughput-supplement.json) reports medians separately; its quotient need not equal the paired ratio. Single-round variance is visible, n=5; no confidence/cold/hot/universal claim.

Both cohorts use C1,64MiB,1MiB blocks,pattern90, write+fsync followed by fresh read+close,1 warmup+5 alternating paired measurements. Payload times come from the same pinned first-party PR43 C `/io` inside actual ordinary official-runc1.5.2 containers; parent/runc startup times are not benchmark timings. Cache residency is **unobserved**, and the result applies to this write-then-read case; no cold-media or independent physical durability claim. Preset barrier semantics and payload content/shape checks are identical for experiment/reference. Each owned file is closed/verified/deleted before next sample, with peak allocation observed.

## Functional and comparison proof

- [183 driver checks](runtime/checks.json), [232 independent checks](independent-post.json), [15 Linux tool guards](final-linux-guards.stderr) passed. Independent collector recalculated medians from48 actual C payload outputs, exact argv/unique pairs/warmup accounting and48 actual metric scrapes. [Overall result](runtime/result.json), [collector argv](post.command.json).
- Node legacy container switch remains OFF. OFF experiment is actual FUSE; ON uses only the independent host switch and an ordinary container bind of that host workspace. Physical Home ext4 source equals host target and ON OCI final source device/inode; actual cover differs from parent FUSE mount. [Before](runtime/binding-before.json), [after](runtime/binding-after.json), [ON OCI](runtime/on-experiment-container.json). It does not bind FUSE to itself or use the legacy native controller.
- All six experiment windows per cohort have positive OFF read/write callbacks and zero ON read/write callback increments. Full34-series deltas retained per sample; other callbacks are not claimed0. Same actual Node ELF/incarnation/mount identity checked after every round.
- Ordinary experiment/reference OCI specs configure UID/GID501, zero capabilities, noNewPrivileges, read-only rootfs and nosuid/nodev workspace flags. All four [specs](on-experiment-spec.json) and cloned eight-input hashes verified independently against [pinned template](rootfs-inputs.json); official runc and payload ELF SHAs unchanged.
- [OFF actual Meta/Node waits](runtime/off-actual-waits.json) captured before next startup clears old live files; [ON waits](runtime/on-actual-waits.json) re-read independently. All four service waits0, eight service/supervisor PIDs and four OCI PIDs gone, runtime list empty; normal mount closure, protected identities and prior install unchanged. No force/lazy cleanup or environment repair.
- Original256MiB ceiling/1GiB floor retained. Recorded peak **248,127,488B**; post allocation101,605,376B and free8,170,201,088B. Only temporary admission ELF copies and normally stopped owned rootfs copies were removed; exact input/clone hashes persist. No VM resize or baseline modification.

Product Meta SHA `4150942fd873b879ab6dc9034904116cf1670904a259d6a081f3b0c1fd1e7343`, Node SHA `9478f3e89905b310689c0727ce0adf28fc11a25444c9634f82055a664069b41d`, reproduced package SHA `c7ea49d7eb59a75275d613f44b17aaebc7b745f6488bcebd9372bc9158f2e386`. [Package provenance](reused-package-proof.json). Original6d timing and b312 callback evidence retain their version/scope; they were not rerun or retroactively given new counters.

## Remaining scope and storage

This completes the two current small **data performance subitems**, not whole G2.13/full ON, standard POSIX or G2.27 release. Current small metadata is next; general authorization/native-FD drain/root changes, mixed append/classic locks/watch and complex reliability remain independently pending, with existing failures retained. Default OFF and G1 historical8/8 unchanged. Old ordinary performance FAIL remains; no repeated score polishing.

Raw: `rust-distributed-memory-store/evidence/afs-delivery/workspace-bind-data-perf-20261007-r1/`. [Outside-source archives/collector provenance](external-artifacts.json) retain binary/transport/result archives and the independent collector; no Python/C/Rust program snapshots, ELF/rootfs or private keys are duplicated in this portable text packet. [SHA256SUMS](SHA256SUMS) covers all packet files except itself.
