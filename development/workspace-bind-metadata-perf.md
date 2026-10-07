# Current workspace: bounded metadata performance

2026-10-07. Plan fixed before execution. Continue G2.13 after the two completed data subitems; do not reopen G1 or repeat data timings. Main is the only development/publication entry.

Reuse product93169c8, its157 compiler inputs, exact qualified release package/ELFs, official runc1.5.2 and the eight-file rootfs template. No Rust/vendor/C changes or rebuild. Ordinary UID/GID501 containers bind the host workspace: OFF actual FUSE, ON independent host workspace bind, reference native ext4. Legacy container controller remains OFF.

Use the pinned `/benchmark` C payload: fresh directory,1000 files of4096 bytes, concurrency1, absolute paths. Six independent phases: create/write/close, stat, read/close, readdir, rename, unlink. Successful payload execution proves file sizes, content and directory count; validate exact JSON shape, operation accounting and timing/latency fields. Close is visibility only; durability and cache residency are unqualified. No runc startup time inside payload phases.

One warmup plus five alternating experiment/reference pairs in each OFF/ON cohort. Each phase reports the median and full range of five reference wall_ns/experiment wall_ns ratios. Preset ON target>=0.90 for each phase; preserve FAIL without tuning or rerunning unchanged inputs. OFF is diagnostic; no historical result relabeling.

Record all34 callback series before/after each sample. Experiment OFF must have positive create/write/read/readdir/rename/unlink/mkdir/rmdir increments; ON must have zero increments for those eight. This proves the selected workload route, not all callbacks or wire counts. Inspect the emptied sample directory and remove only that owned directory normally before the after snapshot.

Reuse existing Linux admission and lifecycle recorder: frozen identities, tools/libraries, source/target/namespace, ports/FUSE/ext4/RAM, protected objects,256MiB allocation ceiling and1GiB free floor. Add explicit benchmark library admission. Normal OCI stop/delete, actual service wait0 and mount/PID closure remain required. Real environment blockers preserve evidence and stop this lane; independent work continues.

Targeted Linux tests guard payload validity, operation/latency accounting, warmup/pair order and independent phase decisions. An independent Linux collector verifies raw payloads, deltas, ratios, receipts and protected identities. Only text results/commands/hashes/index enter Git; archives and program snapshots remain outside source. Next is DFS one-writer/many-reader, with existing baseline qualification limits retained.

## Recorded exit

[Case FAIL and partial data](evidence/20261007-workspace-bind-metadata-perf/README.md): OFF six paired phase diagnostics retained; ON first warmup route witness failed with readdir4, no measured comparison. Allocation traversal inside the global counter window is a concrete confound; attribution remains unverified. No adjusted criterion or rerun.13 Linux tool tests/117 independent evidence checks passed; current metadata performance qualification did not. Next DFS one-writer/many-reader.
