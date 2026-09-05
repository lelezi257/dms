#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

stop_process dms-metrics-host
stop_process dms-node
stop_process dms-meta
echo "dms-metrics-host、dms-node 和 dms-meta 已停止"
