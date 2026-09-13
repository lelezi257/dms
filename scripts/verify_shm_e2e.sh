#!/usr/bin/env bash
set -euo pipefail

# Runs a real three-process local SHM smoke:
#   dms-meta process + dms-node process + SDK example process.
# Intended to be executed inside the Linux dms-dev VM from /workspace/dms/source.

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORK_DIR="${DMS_SHM_E2E_DIR:-$(mktemp -d /tmp/dms-shm-e2e.XXXXXX)}"
RUN_DIR="${WORK_DIR}/run"
LOG_DIR="${WORK_DIR}/logs"
mkdir -p "${RUN_DIR}" "${LOG_DIR}"

pick_port() {
  python3 - <<'PY'
import socket
sock = socket.socket()
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

META_GRPC_ADDRESS="127.0.0.1:$(pick_port)"
META_HEALTH_ADDRESS="127.0.0.1:$(pick_port)"
NODE_HEALTH_ADDRESS="127.0.0.1:$(pick_port)"
WORKER_UDS_PATH="${RUN_DIR}/dms-worker.sock"
META_JOURNAL_DIR="${WORK_DIR}/meta-journal"
mkdir -p "${META_JOURNAL_DIR}"

META_PID=""
NODE_PID=""

cleanup() {
  if [[ -n "${NODE_PID}" ]]; then
    kill "${NODE_PID}" 2>/dev/null || true
  fi
  if [[ -n "${META_PID}" ]]; then
    kill "${META_PID}" 2>/dev/null || true
  fi
  wait "${NODE_PID:-}" 2>/dev/null || true
  wait "${META_PID:-}" 2>/dev/null || true
}
trap cleanup EXIT

wait_health() {
  local address="$1"
  local component="$2"
  local node="$3"
  for _ in $(seq 1 80); do
    if cargo run --quiet --bin dms-health -- health \
      --address "${address}" \
      --expect-component "${component}" \
      --expect-node "${node}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  echo "timed out waiting for ${component} health at ${address}" >&2
  return 1
}

cd "${ROOT_DIR}"
cargo build --workspace

cargo run --quiet --bin dms-meta -- serve \
  --node-id meta-a \
  --health-address "${META_HEALTH_ADDRESS}" \
  --grpc-address "${META_GRPC_ADDRESS}" \
  --journal-dir "${META_JOURNAL_DIR}" \
  >"${LOG_DIR}/dms-meta.log" 2>&1 &
META_PID="$!"
wait_health "${META_HEALTH_ADDRESS}" dms-meta meta-a

cargo run --quiet --bin dms-node -- serve \
  --node-id node-a \
  --health-address "${NODE_HEALTH_ADDRESS}" \
  --worker-uds-path "${WORKER_UDS_PATH}" \
  --meta-endpoint "http://${META_GRPC_ADDRESS}" \
  --arena-capacity-bytes "$((64 * 1024 * 1024))" \
  >"${LOG_DIR}/dms-node.log" 2>&1 &
NODE_PID="$!"
wait_health "${NODE_HEALTH_ADDRESS}" dms-node node-a

DMS_ENDPOINT="unix://${WORKER_UDS_PATH}" \
  cargo run --quiet -p dms-client --example sdk_shared_memory

# The example writes and then opens a read view in the same Region. The first
# slice causes one AcquireRegion + SCM_RIGHTS delivery; the read must reuse the
# SDK mmap cache rather than requesting a second fd.
#
# FD grant is a metric, not a normal log line: Node logs are intentionally kept
# at lifecycle/error granularity, while per-path counters are exported through
# Prometheus text format on the status endpoint.
NODE_METRICS="${LOG_DIR}/dms-node.prom"
curl --fail --silent --show-error "http://${NODE_HEALTH_ADDRESS}/metrics" >"${NODE_METRICS}"

sample_value() {
  local file="$1"
  local sample="$2"
  awk -v sample="${sample}" '
    index($0, sample " ") == 1 { print $NF; found = 1; exit }
    END { if (!found) exit 1 }
  ' "${file}"
}

require_sample_equals() {
  local file="$1"
  local sample="$2"
  local expected="$3"
  local value
  if ! value="$(sample_value "${file}" "${sample}")"; then
    echo "missing metric sample ${sample} in ${file}" >&2
    exit 1
  fi
  if [[ "${value}" != "${expected}" ]]; then
    echo "expected ${sample}=${expected}, got ${value}" >&2
    exit 1
  fi
}

require_sample_equals "${NODE_METRICS}" \
  'dms_node_shm_fd_grants_total{result="issued"}' \
  1
require_sample_equals "${NODE_METRICS}" \
  'dms_node_shm_fd_grants_total{result="claimed"}' \
  1
require_sample_equals "${NODE_METRICS}" \
  'dms_rpc_server_requests_total{method="AcquireRegion",result="ok",service="WorkerService"}' \
  1
require_sample_equals "${NODE_METRICS}" \
  'dms_node_shm_fd_grants_total{result="error"}' \
  0

echo "shm e2e passed fd_grants=1 work_dir=${WORK_DIR}"
