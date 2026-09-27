# AFS fuser patch

This directory vendors `fuser 0.16.0` under its original MIT license. The
`src/` tree matches the crates.io release except for `src/reply.rs` and a
crate-local lint allowance in `src/lib.rs`.

AFS needs independent positive-name and attribute cache lifetimes. Its remote
OwnerFs path caches a successful name-to-inode mapping briefly to avoid a peer
RPC on every path walk, while keeping attributes uncached so a later `stat` or
open can detect changed size and file identity. `ReplyEntry::entry()` in 0.16
uses one TTL for both fields, so the patch adds `entry_with_ttls()` with
separate `entry_ttl` and `attr_ttl` arguments. The method passes them to the
wire encoder in the encoder's required `attr_ttl, entry_ttl` order.

Upstream fuser 0.18 has a similar method, but that release changes the
`Filesystem` API. Replacing this vendor patch should be a deliberate API
migration, not a dependency-only version bump.

`src/lib.rs` allows the upstream 0.16 dead ABI structs and `io_other_error`
Clippy finding, so AFS can retain a strict `-D warnings` gate for product
code without editing unrelated upstream logic.
