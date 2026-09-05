#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

ROOT="$(mktemp -d /tmp/dms-error-codes.XXXXXX)"
RUN_DIR="${ROOT}/run"
LOG_DIR="${ROOT}/log"
META_JOURNAL_DIR="${ROOT}/meta-journal"
BIN_DIR="${CARGO_TARGET_DIR}/release"
EXAMPLE="${BIN_DIR}/examples/sdk_kv"

BASE_PORT=$((23000 + ($$ % 20000)))
META_HEALTH="127.0.0.1:$((BASE_PORT + 1))"
META_GRPC="127.0.0.1:$((BASE_PORT + 2))"
NODE_HEALTH="127.0.0.1:$((BASE_PORT + 3))"
NODE_WORKER="127.0.0.1:$((BASE_PORT + 4))"
ENDPOINT="http://${NODE_WORKER}"
EXPECTED_NODE_ARENA_CODE="33619969"

mkdir -p "${RUN_DIR}" "${LOG_DIR}" "${META_JOURNAL_DIR}"

require_binary() {
  local path="$1"
  if [[ ! -x "${path}" ]]; then
    echo "缺少 ${path}，请先执行 ./scripts/build.sh" >&2
    exit 2
  fi
}

stop_pid_file() {
  local pid_file="$1"
  [[ -r "${pid_file}" ]] || return 0
  local pid
  pid="$(cat "${pid_file}")"
  if [[ "${pid}" =~ ^[0-9]+$ ]]; then
    kill "${pid}" 2>/dev/null || true
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      kill -0 "${pid}" 2>/dev/null || break
      sleep 0.1
    done
  fi
  rm -f "${pid_file}"
}

cleanup() {
  stop_pid_file "${RUN_DIR}/node.pid"
  stop_pid_file "${RUN_DIR}/meta.pid"
  rm -rf "${ROOT}"
}

trap cleanup EXIT

require_binary "${BIN_DIR}/dms-meta"
require_binary "${BIN_DIR}/dms-node"
require_binary "${BIN_DIR}/dms-health"
require_binary "${EXAMPLE}"

echo "== start dedicated meta =="
nohup "${BIN_DIR}/dms-meta" serve \
  --node-id "error-code-meta" \
  --health-address "${META_HEALTH}" \
  --grpc-address "${META_GRPC}" \
  --journal-dir "${META_JOURNAL_DIR}" \
  >"${LOG_DIR}/meta.log" 2>&1 &
echo "$!" >"${RUN_DIR}/meta.pid"

sleep 0.3
"${BIN_DIR}/dms-health" health \
  --address "${META_HEALTH}" \
  --expect-component dms-meta \
  --expect-node error-code-meta

echo "== start small-capacity node =="
nohup "${BIN_DIR}/dms-node" serve \
  --node-id "error-code-node" \
  --health-address "${NODE_HEALTH}" \
  --worker-tcp-address "${NODE_WORKER}" \
  --meta-endpoint "http://${META_GRPC}" \
  --arena-capacity-bytes 4096 \
  >"${LOG_DIR}/node.log" 2>&1 &
echo "$!" >"${RUN_DIR}/node.pid"

sleep 0.8
"${BIN_DIR}/dms-health" health \
  --address "${NODE_HEALTH}" \
  --expect-component dms-node \
  --expect-node error-code-node

echo "== capacity error is native DmsError code =="
CAPACITY_OUTPUT="$(
  DMS_ENDPOINT="${ENDPOINT}" "${EXAMPLE}" expect-capacity-error "error-code/too-large" 4097
)"
echo "${CAPACITY_OUTPUT}"
grep -q "error_code_raw=${EXPECTED_NODE_ARENA_CODE}" <<<"${CAPACITY_OUTPUT}"

echo "== MGET miss is Ok(None) =="
MGET_OUTPUT="$(
  DMS_ENDPOINT="${ENDPOINT}" "${EXAMPLE}" mget-miss "error-code/present" "ok" "error-code/missing"
)"
echo "${MGET_OUTPUT}"
grep -q "missing_is_none=true" <<<"${MGET_OUTPUT}"

echo "error-code E2E ok logs=${LOG_DIR}"
