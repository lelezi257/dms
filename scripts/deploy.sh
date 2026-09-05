#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

require_started_process() {
  local name="$1" pid_file pid
  pid_file="${DMS_RUN_DIR}/${name}.pid"
  pid="$(cat "${pid_file}")"
  if ! kill -0 "${pid}" 2>/dev/null; then
    echo "${name} 启动后已退出；拒绝把旧 listener 的 health 当成新版本就绪" >&2
    tail -n 40 "${DMS_LOG_DIR}/${name}.fallback.log" >&2 || true
    exit 1
  fi
}

declare -A binaries=(
  [dms-node]="${CARGO_TARGET_DIR}/release/dms-node"
  [dms-meta]="${CARGO_TARGET_DIR}/release/dms-meta"
  [dms-health]="${CARGO_TARGET_DIR}/release/dms-health"
  [dms-metrics-host]="${CARGO_TARGET_DIR}/release/examples/metrics_host"
)

for binary in dms-node dms-meta dms-health dms-metrics-host; do
  if [[ ! -x "${binaries[${binary}]}" ]]; then
    echo "缺少 ${binary}，请先执行 ./scripts/build.sh" >&2
    exit 2
  fi
done

mkdir -p "${DMS_BIN_DIR}" "${DMS_LOG_DIR}" "${DMS_RUN_DIR}" "${DMS_META_JOURNAL_DIR}"
stop_process dms-node
stop_process dms-meta
stop_process dms-metrics-host
rm -f "${DMS_META_JOURNAL_DIR}/meta.wal" \
  "${DMS_META_JOURNAL_DIR}/meta.snapshot" \
  "${DMS_META_JOURNAL_DIR}/meta.wal.tmp" \
  "${DMS_META_JOURNAL_DIR}/meta.snapshot.tmp"

for binary in dms-node dms-meta dms-health dms-metrics-host; do
  install -m 0755 "${binaries[${binary}]}" "${DMS_BIN_DIR}/${binary}"
done

# The local reproducible lab starts each run with a clean evidence file. The
# packaged `cluster.sh` deliberately keeps append semantics across restarts.
: >"${DMS_LOG_DIR}/dms-meta.log"
: >"${DMS_LOG_DIR}/dms-node.log"
: >"${DMS_LOG_DIR}/dms-meta.fallback.log"
: >"${DMS_LOG_DIR}/dms-node.fallback.log"

# Meta 必须先监听：Node 启动时会立即注册自己的 Session/epoch。
nohup "${DMS_BIN_DIR}/dms-meta" serve \
  --node-id "meta-${DMS_NODE_ID}" \
  --health-address "0.0.0.0:${DMS_META_PORT}" \
  --grpc-address "0.0.0.0:${DMS_META_GRPC_PORT}" \
  --journal-dir "${DMS_META_JOURNAL_DIR}" \
  --log-level "${DMS_LOG_LEVEL}" \
  --log-format "${DMS_LOG_FORMAT}" \
  --log-path "${DMS_LOG_DIR}/dms-meta.log" \
  --log-max-file-size-bytes "${DMS_LOG_MAX_FILE_SIZE_BYTES}" \
  --log-max-backups "${DMS_LOG_MAX_BACKUPS}" \
  --log-max-age-seconds "${DMS_LOG_MAX_AGE_SECONDS}" \
  --tracing-enabled "${DMS_TRACING_ENABLED}" \
  --tracing-periodic-operations "${DMS_TRACING_PERIODIC_OPERATIONS}" \
  --tracing-otlp-endpoint "${DMS_TRACING_OTLP_ENDPOINT}" \
  --tracing-sample-ratio "${DMS_TRACING_SAMPLE_RATIO}" \
  >/dev/null 2>"${DMS_LOG_DIR}/dms-meta.fallback.log" &
echo "$!" >"${DMS_RUN_DIR}/dms-meta.pid"

sleep 0.2
require_started_process dms-meta

nohup "${DMS_BIN_DIR}/dms-node" serve \
  --node-id "${DMS_NODE_ID}" \
  --health-address "0.0.0.0:${DMS_NODE_PORT}" \
  --worker-tcp-address "0.0.0.0:${DMS_WORKER_PORT}" \
  --worker-uds-path "${DMS_WORKER_UDS}" \
  --meta-endpoint "http://127.0.0.1:${DMS_META_GRPC_PORT}" \
  --staging-ttl-millis "${DMS_STAGING_TTL_MILLIS}" \
  --log-level "${DMS_LOG_LEVEL}" \
  --log-format "${DMS_LOG_FORMAT}" \
  --log-path "${DMS_LOG_DIR}/dms-node.log" \
  --log-max-file-size-bytes "${DMS_LOG_MAX_FILE_SIZE_BYTES}" \
  --log-max-backups "${DMS_LOG_MAX_BACKUPS}" \
  --log-max-age-seconds "${DMS_LOG_MAX_AGE_SECONDS}" \
  --tracing-enabled "${DMS_TRACING_ENABLED}" \
  --tracing-periodic-operations "${DMS_TRACING_PERIODIC_OPERATIONS}" \
  --tracing-otlp-endpoint "${DMS_TRACING_OTLP_ENDPOINT}" \
  --tracing-sample-ratio "${DMS_TRACING_SAMPLE_RATIO}" \
  >/dev/null 2>"${DMS_LOG_DIR}/dms-node.fallback.log" &
echo "$!" >"${DMS_RUN_DIR}/dms-node.pid"

sleep 0.5
require_started_process dms-node
"${SCRIPTS_DIR}/status.sh"

nohup env \
  DMS_ENDPOINT="unix://${DMS_WORKER_UDS}" \
  DMS_CLIENT_METRICS_ADDRESS="0.0.0.0:19400" \
  DMS_TRACING_ENABLED="${DMS_TRACING_ENABLED}" \
  DMS_TRACING_OTLP_ENDPOINT="${DMS_TRACING_OTLP_ENDPOINT}" \
  DMS_TRACING_SAMPLE_RATIO="${DMS_TRACING_SAMPLE_RATIO}" \
  DMS_CLIENT_INSTANCE="${DMS_NODE_ID}-metrics-host" \
  "${DMS_BIN_DIR}/dms-metrics-host" \
  >"${DMS_LOG_DIR}/dms-metrics-host.log" 2>&1 &
echo "$!" >"${DMS_RUN_DIR}/dms-metrics-host.pid"

client_ready=false
for _ in 1 2 3 4 5 6 7 8 9 10; do
  if curl --fail --silent "http://127.0.0.1:19400/healthz" >/dev/null; then
    client_ready=true
    break
  fi
  sleep 0.1
done

if [[ "${client_ready}" != "true" ]]; then
  echo "dms-metrics-host 未就绪，最近日志如下：" >&2
  tail -n 40 "${DMS_LOG_DIR}/dms-metrics-host.log" >&2 || true
  exit 1
fi

echo "部署完成：dms-node + dms-meta"
echo "Worker UDS：unix://${DMS_WORKER_UDS}"
echo "Worker TCP：http://127.0.0.1:${DMS_WORKER_PORT}"
echo "Peer TCP：http://127.0.0.1:${DMS_WORKER_PORT}"
echo "Meta gRPC：http://127.0.0.1:${DMS_META_GRPC_PORT}"
echo "日志目录：${DMS_LOG_DIR}"
echo "Client Metrics：http://127.0.0.1:19400/metrics"
