#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

KEY="${1:-meta/recovery-key}"
VALUE="${2:-durable-v1}"
ENDPOINT="unix://${DMS_WORKER_UDS}"

if [[ ! -x "${CARGO_TARGET_DIR}/release/examples/sdk_kv" ]]; then
  echo "缺少 sdk_kv example，请先执行 ./scripts/build.sh" >&2
  exit 2
fi

echo "== Meta recovery: write through Node before restart =="
DMS_ENDPOINT="${ENDPOINT}" "${CARGO_TARGET_DIR}/release/examples/sdk_kv" set "${KEY}" "${VALUE}"

if [[ ! -s "${DMS_META_JOURNAL_DIR}/meta.wal" && ! -s "${DMS_META_JOURNAL_DIR}/meta.snapshot" ]]; then
  echo "Meta journal directory has no WAL or snapshot after write: ${DMS_META_JOURNAL_DIR}" >&2
  exit 1
fi

watch_count_before="$(grep -c -- "dms-node meta watch established" "${DMS_LOG_DIR}/dms-node.log" 2>/dev/null || true)"

echo "== Meta recovery: restart only dms-meta, keep dms-node alive =="
stop_process dms-meta
nohup "${DMS_BIN_DIR}/dms-meta" serve \
  --node-id "meta-${DMS_NODE_ID}" \
  --health-address "0.0.0.0:${DMS_META_PORT}" \
  --grpc-address "0.0.0.0:${DMS_META_GRPC_PORT}" \
  --journal-dir "${DMS_META_JOURNAL_DIR}" \
  >"${DMS_LOG_DIR}/dms-meta-restarted.log" 2>&1 &
echo "$!" >"${DMS_RUN_DIR}/dms-meta.pid"

for _ in 1 2 3 4 5 6 7 8 9 10; do
  if "${DMS_BIN_DIR}/dms-health" health \
    --address "127.0.0.1:${DMS_META_PORT}" \
    --expect-component dms-meta \
    --expect-node "meta-${DMS_NODE_ID}" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
done

"${DMS_BIN_DIR}/dms-health" health \
  --address "127.0.0.1:${DMS_META_PORT}" \
  --expect-component dms-meta \
  --expect-node "meta-${DMS_NODE_ID}"

for _ in 1 2 3 4 5 6 7 8 9 10; do
  watch_count_after="$(grep -c -- "dms-node meta watch established" "${DMS_LOG_DIR}/dms-node.log" 2>/dev/null || true)"
  if (( watch_count_after > watch_count_before )); then
    break
  fi
  sleep 0.2
done

watch_count_after="$(grep -c -- "dms-node meta watch established" "${DMS_LOG_DIR}/dms-node.log" 2>/dev/null || true)"
if (( watch_count_after <= watch_count_before )); then
  echo "dms-node did not re-establish the Meta watch stream after Meta restart" >&2
  echo "watch_count_before=${watch_count_before} watch_count_after=${watch_count_after}" >&2
  exit 1
fi

echo "== Meta recovery: read through the existing dms-node after Meta restart =="
DMS_ENDPOINT="${ENDPOINT}" "${CARGO_TARGET_DIR}/release/examples/sdk_kv" get "${KEY}" "${VALUE}"

if grep -q -- "dms-node reopened meta session" "${DMS_LOG_DIR}/dms-node.log"; then
  echo "Meta session reopen path observed in dms-node log"
else
  echo "Meta session remained valid from WAL; watch reconnect path observed"
fi

echo "Meta recovery verified key=${KEY} journal_dir=${DMS_META_JOURNAL_DIR} watch_reconnects=$((watch_count_after - watch_count_before))"
