#!/usr/bin/env bash
set -euo pipefail
base=/home/lzc.guest/afs-build
cd "$base/work/owner-eio-v80"
out="$base/evidence/owner-eio-v80/local"
mkdir -p "$out"
export PATH=/home/lzc.guest/.cargo/bin:$PATH CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$base/target"
run() {
    name=$1; shift
    printf '%q ' "$@" > "$out/$name.command"
    printf '\n' >> "$out/$name.command"
    if "$@" > "$out/$name.log" 2>&1; then result=0; else result=$?; fi
    printf '%s\n' "$result" > "$out/$name.exit"
    test "$result" -eq 0
}
run fmt timeout 60 cargo fmt --all -- --check
run owner timeout 240 cargo test --offline --all-features --lib node::vfs::ownerfs::tests:: -- --nocapture
run compile timeout 180 cargo check --offline --all-features --all-targets
