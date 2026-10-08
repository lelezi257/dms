# G2.14 OwnerFs remote READ output ownership

One product cost: keep validated gRPC READ Bytes owned through OwnerFs and into the synchronous public FUSE ReplyData::data call, avoiding the preallocated FUSE output buffer and reply-to-buffer copy. Keep existing FIFO dispatch, native Home root guard, per-handle locking/release serialization, authorization/Home epochs, checksum/shape checks, errno, metrics and prefetch behavior. Local files and RDMA retain allocating read fallback. No DFS/general Backend changes, dependencies or third-party modifications.

1. Add an optional owned read API with slice-compatible default and meaningful allocation-identity, short EOF, closed-handle and protocol-validation regression tests. Capture a Linux RED on the copying implementation.
2. Route only OwnerFs FUSE reads through owned read. gRPC returns validated Bytes; slice API delegates and copies. RDMA retains the original buffer operation; prefetch retains its original short-read semantics and no-RPC metrics.
3. Run targeted Linux GREEN tests and formatting; parent owns affected source gates, frozen identity/build and at most one before/after performance pair with bind ON. No performance or final MooseFS claim from unit tests.
4. Parent retains all source/runtime evidence identities and rejected/blocked history, updates active records and pushes main after the measured disposition.

## Parent frozen measurement exit (before any runtime)

Base main deac154f861c80acf219474b9c5d984425287cd5; production binaries retain prior Node8e1216f4 and b80 Meta15648a87 identities. Existing historic A/ctl services remain protected; no DFS runtime starts.

Exactly one64MiB/C1 bind ON B Home/C remote gRPC pair, baseline then candidate, each one warmup+five formal rounds, logical1MiB reads, same physical inode/full SHA/EOF and physical mincore hot check each round. Guest ext4, default mount caching, same uid501/TLS/config/transport and close barrier; whole open/thread/read/content-check/close wall yields throughput. Per-operation CLOCK_MONOTONIC timestamp ends after pread+count-check and before memcmp; every memcmp still enforced outside the operation interval. Pooled320 observed intervals/phase, nearest-rank p50/p95/p99, preselected p95. Do not compare these narrower latencies to historical oracle-inclusive intervals.

Own-baseline retention requires median throughput>=1.05x AND p95<=1.0x, correctness and normal lifecycle closure. A valid negative result stops this direction without retry. Final matched Moose1.2/.8 remains PENDING. The maintained C timer correction uses probe SHA c87a5fd84384871a69379b572ab512b69b1f0c8a6632aca4a44c72ac3d91e3ee identically both phases; Linux strict C compile,8 positive samples and corrupted-content exit2 are recorded in probe.json.

Owned per-role ceiling ctl128/B256/C256MiB, aggregate640MiB; free floors ctl512/B4096/C4096MiB before launch. Read-only preflight verifies dependencies/package/probe/ELF/config/mount/protected state first. No A use, environment repair, threshold relaxation, new package or full matrix. Real blocker stops affected lane and requests help.
