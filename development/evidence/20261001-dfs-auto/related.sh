#!/usr/bin/env bash
set -euo pipefail
test "$(uname -s)" = Linux
export PATH=/home/lzc.guest/.cargo/bin:$PATH CARGO_TARGET_DIR=/home/lzc.guest/afs-build/target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
root=${1:?}; out=${2:?}
test ! -e "$out"; mkdir -p "$out"; cd "$root"
run() {
 local name=$1; shift
 printf '%q ' "$@" > "$out/$name.command"; printf '\n' >> "$out/$name.command"
 set +e; "$@" > "$out/$name.log" 2>&1; local code=$?; set -e
 printf '%s\n' "$code" > "$out/$name.exit"; tail -n 12 "$out/$name.log"; return "$code"
}
run auto-fallback env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib dfs_auto_unsupported_peer_uses_grpc_before_dispatch -- --ignored --nocapture
run required-unsupported env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib dfs_required_unsupported_peer_never_uses_grpc -- --ignored --nocapture
run canonical timeout 180 cargo test --offline --all-features --lib dfs_rdma_unsupported_reply_contract_accepts_only_canonical_false -- --nocapture
run auto-native env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib dfs_auto_prefers_rdma_replica_and_read_real_verbs -- --ignored --nocapture
run required-native env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib dfs_product_rdma_replica_and_read_real_verbs -- --ignored --nocapture
run owner-native env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --test ownerfs_peer_contract ownerpeerclient_rdma_large_write_fsync_cold_read_roundtrip_preserves_payload -- --ignored --nocapture
