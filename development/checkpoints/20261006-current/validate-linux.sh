#!/usr/bin/env bash
# Revalidate this combined checkpoint; output and Cargo cache must be outside source.
set -euo pipefail
[[ $(uname -s)/$(uname -m) == Linux/aarch64 ]]
source_root=$(cd "$(dirname "$0")/../../.." && pwd)
out=${1:?Usage: validate-linux.sh NEW_OUTPUT_DIR}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:?Set an external Linux Cargo target directory}
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
mkdir "$out"
out=$(cd "$out" && pwd)
cd "$source_root"
python3 development/checkpoints/20261006-current/verify-inputs.py capture "$out/inputs.json"
run() {
  local label=$1
  shift
  printf '%q ' "$@" > "$out/$label.command"
  printf '\n' >> "$out/$label.command"
  local rc=0
  "$@" > "$out/$label.log" 2>&1 || rc=$?
  printf '%s\n' "$rc" > "$out/$label.exit"
  printf '%s exit=%s\n' "$label" "$rc"
  [[ $rc == 0 ]]
}
run fmt timeout 60 cargo fmt --all -- --check
run lib timeout 900 cargo test --locked --offline --all-features -p afs --lib -- --nocapture
run contracts timeout 900 cargo test --locked --offline --all-features --test meta_contract --test node_health_contract --test rest_contract --test vfs_contract --test config_contract --test error_contract --test fuse_contract --test meta_health_capability --test ownerfs_peer_contract -- --nocapture
run error timeout 180 cargo test --locked --offline -p afs-error -- --nocapture
run local-api timeout 300 cargo test --locked --offline --all-features --test local_sdk -- --nocapture
run fuse-build timeout 300 cargo test --locked --offline --all-features --test fuse_contract --no-run --message-format=json
fuse_binary=$(python3 - "$out/fuse-build.log" <<'PY'
import json,sys
rows=[json.loads(s) for s in open(sys.argv[1]) if s.startswith('{')]
paths={r['executable'] for r in rows if r.get('reason')=='compiler-artifact' and r.get('executable') and r['target']['name']=='fuse_contract' and r['profile']['test']}
assert len(paths)==1, paths
print(paths.pop())
PY
)
run fuse sudo timeout 180 "$fuse_binary" --ignored --test-threads=1 --nocapture
run no-features timeout 300 cargo check --locked --offline --no-default-features
run owner-features timeout 300 cargo check --locked --offline --no-default-features --features ownerfs
run dfs-features timeout 300 cargo check --locked --offline --no-default-features --features dfs
run owner-rdma timeout 300 cargo check --locked --offline --no-default-features --features ownerfs,rdma
run dfs-rdma timeout 300 cargo check --locked --offline --no-default-features --features dfs,rdma
run clippy timeout 900 cargo clippy --locked --offline --workspace --all-targets --all-features -- -D warnings
run build timeout 900 cargo build --locked --offline --all-features
run acceptance-drivers timeout 300 python3 -m unittest discover -s development/acceptance -p 'test_*.py'
run trial-config timeout 120 bash scripts/deploy/test-trial-config.sh
run selfcheck-reference timeout 120 bash scripts/deploy/test-selfcheck-reference.sh
run package-reproducible timeout 120 bash scripts/deploy/test-package-reproducible.sh
python3 development/checkpoints/20261006-current/verify-inputs.py verify "$out/inputs.json"
sha256sum "$CARGO_TARGET_DIR/debug/afs-meta" "$CARGO_TARGET_DIR/debug/afs-node" > "$out/binaries.sha256"
printf 'PASS: combined source and tool checkpoint; formal performance NOT_RUN\n'
