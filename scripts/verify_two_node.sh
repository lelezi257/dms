#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

ROOT="${DMS_TWO_NODE_ROOT:-/tmp/dms-two-node-dev}"
BIN_DIR="${CARGO_TARGET_DIR}/release"
EXAMPLE="${BIN_DIR}/examples/sdk_kv"
META_JOURNAL_DIR="${ROOT}/meta-journal"
RUN_DIR="${ROOT}/run"
LOG_DIR="${ROOT}/log"

META_HEALTH="127.0.0.1:19110"
META_GRPC="127.0.0.1:19310"
WRITER_HEALTH="127.0.0.1:19011"
WRITER_WORKER="127.0.0.1:19211"
READER_HEALTH="127.0.0.1:19012"
READER_WORKER="127.0.0.1:19212"

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
  stop_pid_file "${RUN_DIR}/reader.pid"
  stop_pid_file "${RUN_DIR}/writer.pid"
  stop_pid_file "${RUN_DIR}/meta.pid"
}

trap cleanup EXIT

require_binary "${BIN_DIR}/dms-meta"
require_binary "${BIN_DIR}/dms-node"
require_binary "${BIN_DIR}/dms-health"
require_binary "${EXAMPLE}"

cleanup
rm -f "${META_JOURNAL_DIR}/meta.wal" \
  "${META_JOURNAL_DIR}/meta.snapshot" \
  "${META_JOURNAL_DIR}/meta.wal.tmp" \
  "${META_JOURNAL_DIR}/meta.snapshot.tmp"

echo "== start meta =="
nohup "${BIN_DIR}/dms-meta" serve \
  --node-id "p3-meta" \
  --health-address "${META_HEALTH}" \
  --grpc-address "${META_GRPC}" \
  --journal-dir "${META_JOURNAL_DIR}" \
  >"${LOG_DIR}/meta.log" 2>&1 &
echo "$!" >"${RUN_DIR}/meta.pid"

sleep 0.3
"${BIN_DIR}/dms-health" health \
  --address "${META_HEALTH}" \
  --expect-component dms-meta \
  --expect-node p3-meta

echo "== start writer node =="
nohup "${BIN_DIR}/dms-node" serve \
  --node-id "p3-writer" \
  --health-address "${WRITER_HEALTH}" \
  --worker-tcp-address "${WRITER_WORKER}" \
  --meta-endpoint "http://${META_GRPC}" \
  >"${LOG_DIR}/writer.log" 2>&1 &
echo "$!" >"${RUN_DIR}/writer.pid"

echo "== start reader node =="
nohup "${BIN_DIR}/dms-node" serve \
  --node-id "p3-reader" \
  --health-address "${READER_HEALTH}" \
  --worker-tcp-address "${READER_WORKER}" \
  --meta-endpoint "http://${META_GRPC}" \
  >"${LOG_DIR}/reader.log" 2>&1 &
echo "$!" >"${RUN_DIR}/reader.pid"

sleep 0.8
"${BIN_DIR}/dms-health" health \
  --address "${WRITER_HEALTH}" \
  --expect-component dms-node \
  --expect-node p3-writer
"${BIN_DIR}/dms-health" health \
  --address "${READER_HEALTH}" \
  --expect-component dms-node \
  --expect-node p3-reader

KEY="p3/two-node/checkpoint"
VALUE="manifest-from-writer"

echo "== writer set =="
DMS_ENDPOINT="http://${WRITER_WORKER}" "${EXAMPLE}" set "${KEY}" "${VALUE}"

echo "== reader remote-miss get, importing block from writer peer =="
DMS_ENDPOINT="http://${READER_WORKER}" "${EXAMPLE}" get "${KEY}" "${VALUE}"

echo "== stop writer; reader must keep serving imported local block =="
stop_pid_file "${RUN_DIR}/writer.pid"
DMS_ENDPOINT="http://${READER_WORKER}" "${EXAMPLE}" get "${KEY}" "${VALUE}"

grep -q "dms-node meta watch established" "${LOG_DIR}/reader.log"

echo "two-node P3 E2E ok"
echo "logs: ${LOG_DIR}"
