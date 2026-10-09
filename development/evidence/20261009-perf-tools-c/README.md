# C VM matching perf prerequisite — 2026-10-09

G2.14 environment prerequisite **PASS_INSTALL_ONLY**. The user authorized installation after the missing-tool blocker. Installed official Ubuntu ARM64 packages `linux-tools-6.8.0-142` and `linux-tools-6.8.0-142-generic`, both `6.8.0-142.142`; exactly two additions, zero upgrades/removals. Existing older tools remain.

`perf --version` returns 6.8.12; `dpkg -V` and a Linux `perf stat -e cpu-clock -- true` capability check exit zero. Boot identity, kernel 6.8.0-142-generic, complete mount inventory and AFS process inventory match before/after. No restart, data-disk change, service launch or product source/binary change occurred. Root filesystem free space changed from 19,119,546,368 to 19,103,391,744 bytes; apt reported 13.5 MB installed payload.

This closes the selected missing-tool prerequisite, **not** a product function/performance gate. G1 historical8/8 and original G2 counts remain unchanged; prior blocked records and published trial identities remain historical.

[Compact result and raw checksums](result.json) identifies external original commands/output at research-root `evidence/afs-delivery/perf-tools-c-20261009-r1/`. No script, binary or raw mount snapshot is copied into Git.

Next: one bounded current remote READ CPU attribution window, reusing the same64MiB file. The [earlier profile](../20261009-owner-remote-read-perf/README.md) has only22/39 samples; do not rerun its identical sparse case or treat tool availability as performance improvement. Any further diagnostic must freeze its distinct question and stop bound before starting. No new Rust build/test is warranted by this environment-only item.
