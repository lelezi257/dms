#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

echo "== Linux release build =="
uname -a
rustc --version
cargo build --workspace --all-targets --release --locked

echo
echo "构建完成："
sha256sum \
  "${CARGO_TARGET_DIR}/release/dms-node" \
  "${CARGO_TARGET_DIR}/release/dms-meta" \
  "${CARGO_TARGET_DIR}/release/dms-health" \
  "${CARGO_TARGET_DIR}/release/examples/metrics_host" \
  "${CARGO_TARGET_DIR}/release/examples/sdk_kv"
