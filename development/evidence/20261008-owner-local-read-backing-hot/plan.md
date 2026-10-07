# Bounded local read comparison

New prospectively frozen G2.09 backend-hot/default-buffered-client case on existing B/ext4, fixed f03/7bfc product and official Moose4.59.2. Preserve prior client-hot FAIL and repeat NOT_QUALIFIED. Same POSIX flags/preload/payload/C1/block/barrier and replicas; backend hot snapshots and stable identities are required, internal default client caching is separately reported. No product/kernel/VM changes.

1. Check Linux dependencies, exact package/ELFs/helper identities, fresh directories, guest ext4, ports, mounts/processes and2GiB+4GiB capacity before services.
2. Validate new no-touch physical range observer with real Linux guards: header exclusion, unaligned bounds, replacement identity, errors, no FD leak, and repeated cold observation without prefault.
3. Execute one warmup/five alternating pairs only after contract/hash freeze. Retain320 syscall intervals per target; independently recompute median rates and p50/p95/p99. Judge throughput>=1.2 and p95<=.8 separately. First prerequisite failure stops case; never reclassify after results or lower targets.
4. Normally close all owned services/mounts with actual wait receipts and preserve old inventory. Archive only raw text/index/delta, verify restoration on Linux. Update live documents and tracker, Lore commit directly to main and normal push. No full standard suite or Rust rebuild for acceptance-only helper.

mincore is an instant residency snapshot, not a pin or disk/durability guarantee: https://man7.org/linux/man-pages/man2/mincore.2.html . Read-only mmap uses no MAP_POPULATE and never dereferences payload: https://man7.org/linux/man-pages/man2/mmap.2.html .
