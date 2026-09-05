#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
DMS_HOME="$(cd "${SCRIPTS_DIR}/.." && pwd -P)"
CONFIG_FILE="${DMS_CONFIG_FILE:-${DMS_HOME}/config/dms.env}"

if [[ ! -r "${CONFIG_FILE}" ]]; then
  echo "缺少 ${CONFIG_FILE}；先复制 config/dms.env.example 并按本节点修改" >&2
  exit 2
fi

# shellcheck disable=SC1090
source "${CONFIG_FILE}"

BIN_DIR="${DMS_HOME}/bin"
RUN_DIR="${DMS_HOME}/run"
LOG_DIR="${DMS_HOME}/log"
DATA_DIR="${DMS_HOME}/data"
mkdir -p "${RUN_DIR}" "${LOG_DIR}" "${DATA_DIR}/meta-journal"

component="${1:-}"
action="${2:-}"

pid_file() {
  echo "${RUN_DIR}/dms-${1}.pid"
}

expected_exe() {
  case "$1" in
    meta) echo "${BIN_DIR}/dms-meta" ;;
    node) echo "${BIN_DIR}/dms-node" ;;
    client) echo "${BIN_DIR}/dms-metrics-host" ;;
    *) return 1 ;;
  esac
}

pid_state() {
  local name="$1" file pid expected actual
  file="$(pid_file "${name}")"
  [[ -r "${file}" ]] || return 1
  pid="$(cat "${file}")"
  if [[ ! "${pid}" =~ ^[0-9]+$ ]]; then
    echo "${name}: 非法 pid 文件 ${file}" >&2
    return 2
  fi
  if ! kill -0 "${pid}" 2>/dev/null; then
    rm -f "${file}"
    return 1
  fi
  expected="$(readlink -f "$(expected_exe "${name}")")"
  actual="$(readlink -f "/proc/${pid}/exe" 2>/dev/null || true)"
  actual="${actual% (deleted)}"
  if [[ "${actual}" != "${expected}" ]]; then
    echo "${name}: pid ${pid} 不属于本包 ${expected}，实际为 ${actual:-unknown}；拒绝操作" >&2
    return 2
  fi
  return 0
}

is_zombie_pid() {
  local pid="$1" state
  state="$(awk '/^State:/ { print $2; exit }' "/proc/${pid}/status" 2>/dev/null || true)"
  [[ "${state}" == "Z" ]]
}

ensure_not_running() {
  local name="$1" state
  set +e
  pid_state "${name}"
  state="$?"
  set -e
  if [[ "${state}" -eq 0 ]]; then
    echo "${name}: 已运行，未隐式停止；如需重启请先执行 ./scripts/cluster.sh ${name} stop" >&2
    return 1
  fi
  case "${state}" in
    1) return 0 ;;
    *) return 1 ;;
  esac
}

stop_component() {
  local name="$1" file pid state
  file="$(pid_file "${name}")"
  [[ -r "${file}" ]] || return 0
  set +e
  pid_state "${name}"
  state="$?"
  set -e
  if [[ "${state}" -ne 0 ]]; then
    case "${state}" in
      1) return 0 ;;
      *) return 1 ;;
    esac
  fi
  pid="$(cat "${file}")"
  kill "${pid}"
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    kill -0 "${pid}" 2>/dev/null || break
    sleep 0.1
  done
  if kill -0 "${pid}" 2>/dev/null && ! is_zombie_pid "${pid}"; then
    echo "${name}: pid ${pid} 在 TERM 后仍存活；保留 ${file}，未强杀，请人工排查" >&2
    return 1
  fi
  rm -f "${file}"
}

start_meta() {
  ensure_not_running meta
  nohup "${BIN_DIR}/dms-meta" serve \
    --node-id "${DMS_META_NODE_ID}" \
    --health-address "${DMS_META_STATUS_BIND}" \
    --grpc-address "${DMS_META_BIND}" \
    --journal-dir "${DATA_DIR}/meta-journal" \
    --log-level "${DMS_LOG_LEVEL}" \
    --log-format "${DMS_LOG_FORMAT}" \
    --log-path "${LOG_DIR}/dms-meta.log" \
    --log-max-file-size-bytes "${DMS_LOG_MAX_FILE_SIZE_BYTES}" \
    --log-max-backups "${DMS_LOG_MAX_BACKUPS}" \
    --log-max-age-seconds "${DMS_LOG_MAX_AGE_SECONDS}" \
    --tracing-enabled "${DMS_TRACING_ENABLED}" \
    --tracing-periodic-operations "${DMS_TRACING_PERIODIC_OPERATIONS:-false}" \
    --tracing-otlp-endpoint "${DMS_TRACING_OTLP_ENDPOINT}" \
    --tracing-sample-ratio "${DMS_TRACING_SAMPLE_RATIO}" \
    >/dev/null 2>"${LOG_DIR}/dms-meta.fallback.log" &
  echo "$!" >"$(pid_file meta)"
  wait_ready meta
}

start_node() {
  ensure_not_running node
  nohup "${BIN_DIR}/dms-node" serve \
    --node-id "${DMS_NODE_ID}" \
    --health-address "${DMS_NODE_STATUS_BIND}" \
    --worker-tcp-address "${DMS_NODE_IP}:${DMS_WORKER_PORT}" \
    --worker-uds-path "${RUN_DIR}/dms-worker.sock" \
    --meta-endpoint "${DMS_META_ENDPOINT}" \
    --arena-capacity-bytes "${DMS_ARENA_CAPACITY_BYTES}" \
    --staging-ttl-millis "${DMS_STAGING_TTL_MILLIS}" \
    --log-level "${DMS_LOG_LEVEL}" \
    --log-format "${DMS_LOG_FORMAT}" \
    --log-path "${LOG_DIR}/dms-node.log" \
    --log-max-file-size-bytes "${DMS_LOG_MAX_FILE_SIZE_BYTES}" \
    --log-max-backups "${DMS_LOG_MAX_BACKUPS}" \
    --log-max-age-seconds "${DMS_LOG_MAX_AGE_SECONDS}" \
    --tracing-enabled "${DMS_TRACING_ENABLED}" \
    --tracing-periodic-operations "${DMS_TRACING_PERIODIC_OPERATIONS:-false}" \
    --tracing-otlp-endpoint "${DMS_TRACING_OTLP_ENDPOINT}" \
    --tracing-sample-ratio "${DMS_TRACING_SAMPLE_RATIO}" \
    >/dev/null 2>"${LOG_DIR}/dms-node.fallback.log" &
  echo "$!" >"$(pid_file node)"
  wait_ready node
}

start_client() {
  ensure_not_running client
  nohup env \
    DMS_ENDPOINT="${DMS_CLIENT_ENDPOINT}" \
    DMS_CLIENT_METRICS_ADDRESS="${DMS_CLIENT_METRICS_BIND}" \
    DMS_TRACING_ENABLED="${DMS_TRACING_ENABLED}" \
    DMS_TRACING_OTLP_ENDPOINT="${DMS_TRACING_OTLP_ENDPOINT}" \
    DMS_TRACING_SAMPLE_RATIO="${DMS_TRACING_SAMPLE_RATIO}" \
    DMS_CLIENT_INSTANCE="${DMS_NODE_ID}-metrics-host" \
    "${BIN_DIR}/dms-metrics-host" \
    >"${LOG_DIR}/dms-client.log" 2>&1 &
  echo "$!" >"$(pid_file client)"
  wait_ready client
}

probe() {
  local address="$1" component_name="$2" node_id="$3"
  "${BIN_DIR}/dms-health" health \
    --address "${address#0.0.0.0:}" \
    --expect-component "${component_name}" \
    --expect-node "${node_id}"
}

status_component() {
  case "$1" in
    meta) probe "127.0.0.1:${DMS_META_STATUS_BIND##*:}" dms-meta "${DMS_META_NODE_ID}" ;;
    node) probe "127.0.0.1:${DMS_NODE_STATUS_BIND##*:}" dms-node "${DMS_NODE_ID}" ;;
    client) curl --fail --silent --show-error "http://127.0.0.1:${DMS_CLIENT_METRICS_BIND##*:}/healthz" ;;
    *) return 2 ;;
  esac
}

wait_ready() {
  local name="$1" pid
  pid="$(cat "$(pid_file "${name}")")"
  for _ in $(seq 1 50); do
    if status_component "${name}" >/dev/null 2>&1; then
      echo "${name}: ready"
      return 0
    fi
    if ! kill -0 "${pid}" 2>/dev/null; then
      echo "${name}: 启动后进程退出；查看 ${LOG_DIR}/dms-${name}.log 和 fallback 日志" >&2
      return 1
    fi
    sleep 0.2
  done
  echo "${name}: 进程已启动但未在超时内 ready；查看 ${LOG_DIR}/dms-${name}.log" >&2
  return 1
}

case "${component}:${action}" in
  meta:start) start_meta ;;
  node:start) start_node ;;
  client:start) start_client ;;
  meta:status) status_component meta ;;
  node:status) status_component node ;;
  client:status) status_component client ;;
  meta:stop) stop_component meta ;;
  node:stop) stop_component node ;;
  client:stop) stop_component client ;;
  all:stop)
    stop_component client
    stop_component node
    stop_component meta
    ;;
  *)
    echo "usage: ./scripts/cluster.sh {meta|node|client} {start|status|stop} | all stop" >&2
    exit 2
    ;;
esac
