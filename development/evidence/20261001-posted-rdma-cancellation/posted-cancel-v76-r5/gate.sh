#!/usr/bin/env bash
set -euo pipefail
base=/home/lzc.guest/afs-build
cd "$base/work/posted-cancel-v76-r2"
out="$base/evidence/posted-cancel-v76-r5/full"
mkdir -p "$out/gate" "$out/features"
export PATH=/home/lzc.guest/.cargo/bin:$PATH CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$base/target"
run() {
    name=$1
    shift
    printf '%q ' "$@" > "$out/$name.command"
    printf '\n' >> "$out/$name.command"
    if "$@" > "$out/$name.log" 2>&1; then result=0; else result=$?; fi
    printf '%s\n' "$result" > "$out/$name.exit"
    test "$result" -eq 0
}
run gate/fmt timeout 60 cargo fmt --all -- --check
run gate/clippy timeout 240 cargo clippy --offline --workspace --all-targets --all-features -- -D warnings
run gate/lib timeout 240 cargo test --offline --all-features --lib -- --nocapture
run gate/contracts timeout 180 cargo test --offline --all-features --test config_contract --test error_contract --test fuse_contract --test meta_contract --test ownerfs_peer_contract --test rest_contract --test vfs_contract -- --nocapture
run gate/error timeout 180 cargo test --offline -p afs-error -- --nocapture
run gate/local-api timeout 180 cargo test --offline --all-features --test local_sdk -- --nocapture
run gate/fuse-build timeout 180 cargo test --offline --all-features --test fuse_contract --no-run
binary=$(sed -n 's/.*Executable tests\/fuse_contract.rs (\(.*\))/\1/p' "$out/gate/fuse-build.log")
test -n "$binary"
run gate/fuse sudo timeout 120 "$binary" --ignored --test-threads=1 --nocapture
run features/no-features timeout 120 cargo check --offline --no-default-features
run features/owner-features timeout 120 cargo check --offline --no-default-features --features ownerfs
run features/dfs-features timeout 120 cargo check --offline --no-default-features --features dfs
run features/owner-rdma timeout 120 cargo check --offline --no-default-features --features ownerfs,rdma
run features/dfs-rdma timeout 120 cargo check --offline --no-default-features --features dfs,rdma
run gate/build timeout 180 cargo build --offline --all-features --bin afs-node --bin afs-meta
