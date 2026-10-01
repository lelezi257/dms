#!/usr/bin/env bash
# Linux build VM only. Each output directory records one frozen-input selection.
set -euo pipefail
test "$(uname -s)" = Linux
export PATH=/home/lzc.guest/.cargo/bin:$PATH
export CARGO_TARGET_DIR=/home/lzc.guest/afs-build/target
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
ROOT=${1:?source root}
OUT=${2:?new output directory}
MODE=${3:?compile, local, related, features or full}
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
    local status=$?
    set -e
    printf '%s\n' "$status" > "$OUT/$name.exit"
    tail -n 9 "$OUT/$name.log"
    return "$status"
}
case "$MODE" in
    compile)
        run compile timeout 180 cargo test --offline --all-features --test ownerfs_peer_contract --no-run --message-format=json
        ;;
    local)
        run original timeout 180 cargo test --offline --all-features --test ownerfs_peer_contract ownerfiles_rejects_rdma_plane_without_negotiated_owner_session_before_write -- --exact --nocapture
        run owner-contract timeout 180 cargo test --offline --all-features --test ownerfs_peer_contract -- --nocapture
        run control timeout 180 cargo test --offline --all-features --lib node::rpc::control::tests -- --nocapture
        run data timeout 180 cargo test --offline --all-features --lib node::rpc::data:: -- --nocapture
        run peer timeout 180 cargo test --offline --all-features --lib node::rpc::peer::tests -- --nocapture
        ;;
    related)
        run owner-vfs timeout 240 cargo test --offline --all-features --lib node::vfs::ownerfs:: -- --nocapture
        run dfs-vfs timeout 240 cargo test --offline --all-features --lib node::vfs::dfs:: -- --nocapture
        run replication timeout 180 cargo test --offline --all-features --lib node::replication:: -- --nocapture
        run meta-contract timeout 180 cargo test --offline --all-features --test meta_contract -- --nocapture
        ;;
    features)
        run no-features timeout 120 cargo check --offline --no-default-features
        run owner-features timeout 120 cargo check --offline --no-default-features --features ownerfs
        run dfs-features timeout 120 cargo check --offline --no-default-features --features dfs
        run owner-rdma timeout 120 cargo check --offline --no-default-features --features ownerfs,rdma
        run dfs-rdma timeout 120 cargo check --offline --no-default-features --features dfs,rdma
        ;;
    full)
        run fmt timeout 60 cargo fmt --all -- --check
        run clippy timeout 240 cargo clippy --offline --workspace --all-targets --all-features -- -D warnings
        run lib timeout 240 cargo test --offline --all-features --lib -- --nocapture
        run error timeout 180 cargo test --offline -p afs-error -- --nocapture
        run contracts timeout 180 cargo test --offline --all-features --test config_contract --test error_contract --test fuse_contract --test meta_contract --test ownerfs_peer_contract --test rest_contract --test vfs_contract -- --nocapture
        run local-api timeout 180 cargo test --offline --all-features --test local_sdk -- --nocapture
        run fuse-build timeout 120 cargo test --offline --all-features --test fuse_contract --no-run --message-format=json
        fuse_harness=$(python3 - "$OUT/fuse-build.log" <<'PY'
import json,sys
items=[]
for line in open(sys.argv[1]):
    if not line.startswith('{'): continue
    item=json.loads(line)
    if item.get('reason')=='compiler-artifact' and item.get('target',{}).get('name')=='fuse_contract' and item.get('executable'): items.append(item['executable'])
assert len(items)==1,items
print(items[0])
PY
)
        run fuse sudo timeout 120 "$fuse_harness" --ignored --test-threads=1 --nocapture
        run build timeout 180 cargo build --offline --all-features --bins
        ;;
    *) exit 2 ;;
esac
