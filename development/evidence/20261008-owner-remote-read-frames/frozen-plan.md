# G2.14 Owner HTTP/2 receive-frame experiment

One product cost: Owner-specific peer channel advertises 256KiB receive frames, instead of HTTP/2 default16KiB. Existing receive windows are already2MiB and remain unchanged. Common/DFS transport, long-lock wait, mTLS, deadlines, grants, identities, shape/checksum, freshness and barriers remain unchanged.

Add separately keyed Owner profile inside existing epoch-fenced bounded pool; regression covers profile isolation and cross-profile epoch eviction; use real Home mTLS contract through pool including full1MiB read and normal release. Existing malformed/permission tests reused. Rust only Linux: targeted tests, default/RDMA check, affected Clippy/fmt, release.

Exactly one64MiB/C1 corrected-probe pair,1warmup+5formal each, B real ext4 Home bindON / C remote FUSE / ctl local-file Meta. Retained8e1216f4 baseline, same payload/inode/config/mTLS/hot physical pages. Require throughput ratio>=1.05 AND independent pooled operation p95 ratio<=1.0 with complete content/EOF/permission/native freshness/actual childwait0 closure. No matched Moose final claim. Negative valid result restores product files and stops this direction. No A use, environment repair or package cycle.
