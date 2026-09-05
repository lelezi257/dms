#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
if [[ "${DMS_DEV_ENV:-}" == "1" && -f "${SCRIPTS_DIR}/_common.sh" ]]; then
  source "${SCRIPTS_DIR}/_common.sh"
else
  DMS_SOURCE_DIR="$(cd "${SCRIPTS_DIR}/.." && pwd -P)"
  CONFIG_FILE="${DMS_CONFIG_FILE:-${DMS_SOURCE_DIR}/config/dms.env}"
  if [[ -r "${CONFIG_FILE}" ]]; then
    # shellcheck disable=SC1090
    source "${CONFIG_FILE}"
  fi
fi

MODE="${1:-single}"
TARGET_FILE="${DMS_SOURCE_DIR}/infra/observability/prometheus/file_sd/targets.json"
TMP_FILE="${TARGET_FILE}.tmp"
SINGLE_NODE_BIND="${DMS_NODE_STATUS_BIND:-}"
SINGLE_NODE_PORT="${SINGLE_NODE_BIND##*:}"
SINGLE_NODE_PORT="${SINGLE_NODE_PORT:-19000}"
SINGLE_META_BIND="${DMS_META_STATUS_BIND:-}"
SINGLE_META_PORT="${SINGLE_META_BIND##*:}"
SINGLE_META_PORT="${SINGLE_META_PORT:-19100}"
SINGLE_CLIENT_BIND="${DMS_CLIENT_METRICS_BIND:-}"
SINGLE_CLIENT_PORT="${SINGLE_CLIENT_BIND##*:}"
SINGLE_CLIENT_PORT="${SINGLE_CLIENT_PORT:-19400}"

case "${MODE}" in
  single)
    cat >"${TMP_FILE}" <<JSON
[
  {"targets":["host.docker.internal:${SINGLE_NODE_PORT}"],"labels":{"job":"dms-node","instance":"dms-dev-node","component":"node"}},
  {"targets":["host.docker.internal:${SINGLE_META_PORT}"],"labels":{"job":"dms-meta","instance":"dms-dev-meta","component":"meta"}},
  {"targets":["host.docker.internal:${SINGLE_CLIENT_PORT}"],"labels":{"job":"dms-client","instance":"dms-dev-client","component":"client"}}
]
JSON
    ;;
  three)
    N1_ADDRESS="${2:?usage: metrics-targets.sh three N1_IP N2_IP N3_IP}"
    N2_ADDRESS="${3:?usage: metrics-targets.sh three N1_IP N2_IP N3_IP}"
    N3_ADDRESS="${4:?usage: metrics-targets.sh three N1_IP N2_IP N3_IP}"
    cat >"${TMP_FILE}" <<JSON
[
  {"targets":["${N1_ADDRESS}:19000"],"labels":{"job":"dms-node","instance":"n1-node","component":"node"}},
  {"targets":["${N2_ADDRESS}:19000"],"labels":{"job":"dms-node","instance":"n2-node","component":"node"}},
  {"targets":["${N3_ADDRESS}:19000"],"labels":{"job":"dms-node","instance":"n3-node","component":"node"}},
  {"targets":["${N1_ADDRESS}:19100"],"labels":{"job":"dms-meta","instance":"n1-meta","component":"meta"}},
  {"targets":["${N1_ADDRESS}:19400"],"labels":{"job":"dms-client","instance":"n1-client","component":"client"}}
]
JSON
    ;;
  *)
    echo "unknown target mode: ${MODE}; expected single or three" >&2
    exit 2
    ;;
esac

mv "${TMP_FILE}" "${TARGET_FILE}"
echo "Prometheus targets updated: ${TARGET_FILE} (${MODE})"
