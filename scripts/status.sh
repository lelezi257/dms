#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

if [[ ! -x "${DMS_BIN_DIR}/dms-health" ]]; then
  echo "尚未部署，请先执行 ./scripts/deploy.sh" >&2
  exit 2
fi

"${DMS_BIN_DIR}/dms-health" health \
  --address "127.0.0.1:${DMS_NODE_PORT}" \
  --expect-component dms-node \
  --expect-node "${DMS_NODE_ID}"

"${DMS_BIN_DIR}/dms-health" health \
  --address "127.0.0.1:${DMS_META_PORT}" \
  --expect-component dms-meta \
  --expect-node "meta-${DMS_NODE_ID}"
