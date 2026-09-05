#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

ROOT="${DMS_REPAIR_ROOT:-/tmp/dms-repair-dev}"
BIN_DIR="${CARGO_TARGET_DIR}/release"
EXAMPLE="${BIN_DIR}/examples/sdk_kv"
META_JOURNAL_DIR="${ROOT}/meta-journal"
RUN_DIR="${ROOT}/run"
LOG_DIR="${ROOT}/log"

META_HEALTH="127.0.0.1:19410"
META_GRPC="127.0.0.1:19420"
NODE1_HEALTH="127.0.0.1:19411"
NODE1_WORKER="127.0.0.1:19421"
NODE2_HEALTH="127.0.0.1:19412"
NODE2_WORKER="127.0.0.1:19422"
NODE3_HEALTH="127.0.0.1:19413"
NODE3_WORKER="127.0.0.1:19423"

mkdir -p "${RUN_DIR}" "${LOG_DIR}" "${META_JOURNAL_DIR}"

require_binary() {
  [[ -x "$1" ]] || { echo "缺少 $1，请先执行 ./scripts/build.sh" >&2; exit 2; }
}

stop_pid_file() {
  local pid_file="$1"
  [[ -r "${pid_file}" ]] || return 0
  local pid
  pid="$(<"${pid_file}")"
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
  stop_pid_file "${RUN_DIR}/node3.pid"
  stop_pid_file "${RUN_DIR}/node2.pid"
  stop_pid_file "${RUN_DIR}/node1.pid"
  stop_pid_file "${RUN_DIR}/meta.pid"
}
trap cleanup EXIT

for binary in dms-meta dms-node dms-health; do
  require_binary "${BIN_DIR}/${binary}"
done
require_binary "${EXAMPLE}"

cleanup
rm -f "${META_JOURNAL_DIR}/meta.wal" \
  "${META_JOURNAL_DIR}/meta.snapshot" \
  "${META_JOURNAL_DIR}/meta.wal.tmp" \
  "${META_JOURNAL_DIR}/meta.snapshot.tmp"

nohup "${BIN_DIR}/dms-meta" serve \
  --node-id repair-meta \
  --health-address "${META_HEALTH}" \
  --grpc-address "${META_GRPC}" \
  --journal-dir "${META_JOURNAL_DIR}" \
  >"${LOG_DIR}/meta.log" 2>&1 &
echo "$!" >"${RUN_DIR}/meta.pid"

start_node() {
  local id="$1" health="$2" worker="$3"
  nohup "${BIN_DIR}/dms-node" serve \
    --node-id "${id}" \
    --health-address "${health}" \
    --worker-tcp-address "${worker}" \
    --meta-endpoint "http://${META_GRPC}" \
    >"${LOG_DIR}/${id}.log" 2>&1 &
  echo "$!" >"${RUN_DIR}/${id}.pid"
}

sleep 0.3
"${BIN_DIR}/dms-health" health --address "${META_HEALTH}" --expect-component dms-meta --expect-node repair-meta
start_node node1 "${NODE1_HEALTH}" "${NODE1_WORKER}"
start_node node2 "${NODE2_HEALTH}" "${NODE2_WORKER}"
start_node node3 "${NODE3_HEALTH}" "${NODE3_WORKER}"
sleep 1

KEY="repair/checkpoint"
VALUE="repairable-manifest"
DMS_ENDPOINT="http://${NODE1_WORKER}" "${EXAMPLE}" set "${KEY}" "${VALUE}"
# A read-through import on node2 explicitly establishes a two-copy policy.
DMS_ENDPOINT="http://${NODE2_WORKER}" "${EXAMPLE}" get "${KEY}" "${VALUE}"

stop_pid_file "${RUN_DIR}/node1.pid"
echo "等待 node1 的 30s lease 过期，并由 Meta 定向修复到 node3..."
deadline=$((SECONDS + 45))
until grep -q "dms-node repair applied" "${LOG_DIR}/node3.log"; do
  if (( SECONDS >= deadline )); then
    echo "repair timeout" >&2
    tail -n 80 "${LOG_DIR}/meta.log" >&2 || true
    tail -n 80 "${LOG_DIR}/node3.log" >&2 || true
    exit 1
  fi
  sleep 1
done

# Once the replacement is active and reported, the only original live source
# may disappear; node3 must still read from its local Arena.
stop_pid_file "${RUN_DIR}/node2.pid"
DMS_ENDPOINT="http://${NODE3_WORKER}" "${EXAMPLE}" get "${KEY}" "${VALUE}"

echo "three-node repair E2E ok"
echo "logs: ${LOG_DIR}"
