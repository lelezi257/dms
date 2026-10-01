#!/usr/bin/env bash
set -euo pipefail
test "$(uname -s)" = Linux
export PATH=/home/lzc.guest/.cargo/bin:$PATH
export CARGO_TARGET_DIR=/home/lzc.guest/afs-build/target
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
ROOT=${1:?frozen source}
OUT=${2:?new output}
MODE=${3:?original,local,native or full}
test ! -e "$OUT"
mkdir -p "$OUT"
cd "$ROOT"
run() {
    local name=$1
    shift
    printf '%q ' "$@" > "$OUT/$name.command"
    printf '\n' >> "$OUT/$name.command"
    set +e
    "$@" > "$OUT/$name.log" 2>&1
    local result=$?
    set -e
    printf '%s\n' "$result" > "$OUT/$name.exit"
    tail -n 8 "$OUT/$name.log"
    return "$result"
}
case "$MODE" in
original)
    run original env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib closed_rdma_endpoints_retain_admission_until_last_owner_drops -- --ignored --nocapture
    ;;
local)
    run control timeout 180 cargo test --offline --all-features --lib node::rpc::control::tests -- --nocapture
    run original env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib closed_rdma_endpoints_retain_admission_until_last_owner_drops -- --ignored --nocapture
    run data timeout 180 cargo test --offline --all-features --lib node::rpc::data:: -- --nocapture
    run peer timeout 180 cargo test --offline --all-features --lib node::rpc::peer::tests -- --nocapture
    run contracts timeout 180 cargo test --offline --all-features --test ownerfs_peer_contract -- --nocapture
    ;;
native)
    run dfs env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib dfs_product_rdma_replica_and_read_real_verbs -- --ignored --nocapture
    run owner env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --test ownerfs_peer_contract ownerpeerclient_rdma_large_write_fsync_cold_read_roundtrip_preserves_payload -- --ignored --nocapture
    run lifecycle env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --test rdma_lifecycle -- --ignored --test-threads=1 --nocapture
    ;;
full)
    # Existing unchanged full source gate; feature consumers are checked here too.
    bash "$ROOT/development/evidence/20261001-owner-metrics/source-gate.sh" "$ROOT" "$OUT/gate" full
    bash "$ROOT/development/evidence/20261001-owner-metrics/source-gate.sh" "$ROOT" "$OUT/features" features
    ;;
*) exit 2 ;;
esac
