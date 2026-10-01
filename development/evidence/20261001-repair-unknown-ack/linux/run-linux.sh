#!/usr/bin/env bash
# Build VM only; local feedback and final batch gate are separate invocations.
set -euo pipefail
test "$(uname -s)" = Linux
export PATH=/home/lzc.guest/.cargo/bin:$PATH
export CARGO_TARGET_DIR=/home/lzc.guest/afs-build/target
export CARGO_BUILD_JOBS=2
export CARGO_INCREMENTAL=0
ROOT=/home/lzc.guest/afs-build/work/root-merged-v63-r1
OUT=/home/lzc.guest/afs-build/probes/unknown-ack-v63-r1
cd "$ROOT"
mkdir -p "$OUT/${1:?choose local, lint-fix or full}"
run() {
    local name=$1
    shift
    local log="$OUT/$MODE/$name"
    printf '%q ' "$@" > "$log.command"
    printf '\n' >> "$log.command"
    set +e
    "$@" > "$log.log" 2>&1
    local status=$?
    set -e
    printf '%s\n' "$status" > "$log.exit"
    tail -n 8 "$log.log"
    return "$status"
}
MODE=$1
case "$MODE" in
    lint-fix)
        run clippy timeout 240 cargo clippy --offline --all-targets --all-features -- -D warnings
        run wire timeout 240 cargo test --offline --all-features --lib node::rpc::data::tests::repair_worker_replays_real_grpc_claim_and_report_after_lost_ack -- --exact --nocapture
        run fmt timeout 60 cargo fmt --all -- --check
        ;;
    local)
        run wire timeout 240 cargo test --offline --all-features --lib node::rpc::data::tests::repair_worker_replays_real_grpc_claim_and_report_after_lost_ack -- --exact --nocapture
        run data timeout 180 cargo test --offline --all-features --lib node::rpc::data::tests -- --nocapture
        run replication timeout 180 cargo test --offline --all-features --lib node::replication::tests -- --nocapture
        run meta-contract timeout 180 cargo test --offline --all-features --test meta_contract -- --nocapture
        run fmt timeout 60 cargo fmt --all -- --check
        run clippy timeout 240 cargo clippy --offline --all-targets --all-features -- -D warnings
        ;;
    full)
        run fmt timeout 60 cargo fmt --all -- --check
        run clippy timeout 240 cargo clippy --offline --workspace --all-targets --all-features -- -D warnings
        run lib timeout 240 cargo test --offline --all-features --lib -- --nocapture
        run error timeout 180 cargo test --offline -p afs-error -- --nocapture
        run contracts timeout 180 cargo test --offline --all-features --test config_contract --test error_contract --test fuse_contract --test meta_contract --test ownerfs_peer_contract --test rest_contract --test vfs_contract -- --nocapture
        run local-api timeout 180 cargo test --offline --all-features --test local_sdk -- --nocapture
        run fuse-build timeout 120 cargo test --offline --all-features --test fuse_contract --no-run --message-format=json
        fuse_harness=$(python3 - "$OUT/full/fuse-build.log" <<'PY'
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
        run no-features timeout 120 cargo check --offline --no-default-features
        run owner-features timeout 120 cargo check --offline --no-default-features --features ownerfs
        run dfs-features timeout 120 cargo check --offline --no-default-features --features dfs
        run build timeout 180 cargo build --offline --all-features --bins
        ;;
    *) exit 2 ;;
esac
