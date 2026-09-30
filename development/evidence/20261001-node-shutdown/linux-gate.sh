#!/usr/bin/env bash
set -euo pipefail
test "$(uname -s)" = Linux
export PATH=/home/lzc.guest/.cargo/bin:$PATH
export CARGO_TARGET_DIR=/home/lzc.guest/afs-build/target
export CARGO_BUILD_JOBS=2
export CARGO_INCREMENTAL=0
cd /home/lzc.guest/afs-build/work/root-merged-v48
python3 /var/tmp/capture-afs-compile-inputs.py /var/tmp/v48-host-source-hashes.json "$PWD"
timeout 60 cargo fmt --all -- --check
timeout 240 cargo clippy --workspace --all-targets --all-features -- -D warnings
timeout 300 cargo test --all-features --lib -- --nocapture
timeout 180 cargo test --all-features --test local_sdk -- --nocapture
timeout 180 cargo test -p afs-error -- --nocapture
timeout 180 cargo test --all-features --test config_contract --test error_contract --test fuse_contract --test meta_contract --test ownerfs_peer_contract --test rest_contract --test vfs_contract -- --nocapture
timeout 120 cargo test --all-features --test fuse_contract --no-run --message-format=json > /home/lzc.guest/afs-build/work/root-merged-v48-fuse-build.jsonl
fuse_harness=$(python3 - <<'PY'
import json
from pathlib import Path
items = []
for line in Path('/home/lzc.guest/afs-build/work/root-merged-v48-fuse-build.jsonl').read_text().splitlines():
    item = json.loads(line)
    if item.get('reason') == 'compiler-artifact' and item.get('target', {}).get('name') == 'fuse_contract' and item.get('executable'):
        items.append(item['executable'])
assert len(items) == 1, items
print(items[0])
PY
)
sudo timeout 120 "$fuse_harness" --ignored --test-threads=1 --nocapture
timeout 120 cargo check --no-default-features
timeout 120 cargo check --no-default-features --features ownerfs
timeout 120 cargo check --no-default-features --features dfs
timeout 180 cargo build --all-features --bins
mkdir /home/lzc.guest/afs-build/artifacts/v48-qualified
cp "$CARGO_TARGET_DIR/debug/afs-node" "$CARGO_TARGET_DIR/debug/afs-meta" /home/lzc.guest/afs-build/artifacts/v48-qualified/
sha256sum /home/lzc.guest/afs-build/artifacts/v48-qualified/afs-node /home/lzc.guest/afs-build/artifacts/v48-qualified/afs-meta
