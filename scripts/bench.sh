#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

BENCH_OPS="${DMS_BENCH_OPS:-200}"
BENCH_WARMUP_OPS="${DMS_BENCH_WARMUP_OPS:-20}"
BENCH_ENDPOINT="${DMS_BENCH_ENDPOINT:-http://127.0.0.1:${DMS_WORKER_PORT}}"
BENCH_PROJECT_DIR="$(cd "${DMS_SOURCE_DIR}/.." && pwd -P)"
BENCH_RESULT_DIR="${DMS_BENCH_RESULT_DIR:-${BENCH_PROJECT_DIR}/evidence/$(date -u +%Y-%m-%dT%H%M%SZ)-p4-benchmark}"
BENCH_OUTPUT="${BENCH_RESULT_DIR}/dms-bench.json"

mkdir -p "${BENCH_RESULT_DIR}"

echo "== 1/3 build benchmark runner =="
cargo build -p dms-bench --release --locked

echo "== 2/3 deploy single-node baseline =="
"${SCRIPTS_DIR}/deploy.sh"

echo "== 3/3 run benchmark =="
DMS_ENDPOINT="${BENCH_ENDPOINT}" \
  "${CARGO_TARGET_DIR}/release/dms-bench" \
  --endpoint "${BENCH_ENDPOINT}" \
  --ops "${BENCH_OPS}" \
  --warmup-ops "${BENCH_WARMUP_OPS}" \
  --output "${BENCH_OUTPUT}"

echo "benchmark result: ${BENCH_OUTPUT}"
