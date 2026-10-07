# DFS single-writer small write observation

2026-10-07, independent G2.23 subitem on main. Preserve G1 historical8/8,
published7e6e00a6/map151a2c6d/packagec3bb5a30 and all previous results.

One writer A,64MiB per file,1MiB blocks,C1,one warmup and five measured new-file
writes. Each round uses a different counter generation: first8 bytes of each
block encode `(generation <<32) | block_index`, generation1..6. All96 distinct
4MiB contents must differ across rounds. This avoids cross-round deduplication
and does not discard earlier files to manufacture a smaller allocation.

Before editing, inspect existing C probe/driver tests, fixed three-copy fixture
and read-only Meta/catalog observers. Keep the legacy C CLI/dataset unchanged;
add optional bounded generation and Linux negative controls. Add a maintained
single-round Python writer/read-check and exact-six-round summarizer, reusing
existing candidate/path/content/EOF helpers. No Rust/vendor change or product
rebuild. External observer adaptation selects the exact current file version
by its directory entry/inode/head version; never select an arbitrary layout.

Use fresh roots on existing ctl/A/B/C ARM64 Linux/ext4 VMs; local-file/gRPC,
both workspace switches OFF,desired/synchronous copies/minimum distinct node
and configured domain identities all3,local-copy required. The six retained
files require1152MiB physical chunk bytes plus about388MiB package inputs:
declare an aggregate **2GiB** allocation ceiling before this run, retaining
the existing per-role1GiB ceiling and1GiB backing-volume reserve. This new
case budget is not an alteration of any previous failed qualification. Logs
are bounded inputs, not the reason for the data allocation. Three VMs share
one physical host; no claim of three physical failure domains.

Admit dependencies/compiler for the C probe once, then actual exact package,
tools/config/TLS/ports/RAM/mounts/capacity and pre-existing inventories before
services start. Create root-owned results parents during preparation. Every
write times C open/write/fdatasync/close; directory fsync/full SHA/EOF and B/C
fresh-open readback happen outside that timing. After every round, independently
verify16 chunks ×3 ReadyDurable CopyRecords and48 physical byte/SHA copies
before the next write. Preserve all six files, raw timings and barrier timings.

One bounded runtime attempt;60s operations and30s controls. Stop an affected
lane on genuine environment blockage, preserve proof and ask for help. No
environment repair/expansion or unchanged/score-based rerun. Normal final
closure requires four actual wait0,eight child/supervisor PIDs gone,exact
FUSE/UDS and full original inventory,plus budget checks.

Five write samples/median constitute current data, not a qualified3FS parity
result. Original complete ReplicaAck and power-loss durability are not exposed
by persisted derived CopyRecords. Cache/RPC placement are unobserved. G2.23
formal comparison,G2.27/full bind and overall goal remain open. Publish compact
commands/versions/results/hashes/indices with Lore on main; keep binaries,
payloads,TLS,full archives and observer sources outside the code evidence tree.
