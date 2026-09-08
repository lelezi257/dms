#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
if [[ "${DMS_DEV_ENV:-}" == "1" && -f "${SCRIPTS_DIR}/_common.sh" ]]; then
  source "${SCRIPTS_DIR}/_common.sh"
  DEFAULT_NODE_PORT="${DMS_NODE_PORT}"
  DEFAULT_META_PORT="${DMS_META_PORT}"
else
  DMS_SOURCE_DIR="$(cd "${SCRIPTS_DIR}/.." && pwd -P)"
  CONFIG_FILE="${DMS_CONFIG_FILE:-${DMS_SOURCE_DIR}/config/dms.env}"
  if [[ ! -r "${CONFIG_FILE}" ]]; then
    echo "缺少 ${CONFIG_FILE}；先复制 config/dms.env.example 并按本节点修改" >&2
    exit 2
  fi
  # shellcheck disable=SC1090
  source "${CONFIG_FILE}"
  DEFAULT_NODE_PORT="${DMS_NODE_STATUS_BIND##*:}"
  DEFAULT_META_PORT="${DMS_META_STATUS_BIND##*:}"
fi

TOPOLOGY="single"
if [[ "${1:-}" == "--topology" ]]; then
  TOPOLOGY="${2:?missing topology after --topology}"
  shift 2
fi

CLIENT_ADDRESS="${DMS_CLIENT_METRICS_ADDRESS:-127.0.0.1:19400}"
NODE_ADDRESS="${DMS_NODE_METRICS_ADDRESS:-127.0.0.1:${DEFAULT_NODE_PORT}}"
META_ADDRESS="${DMS_META_METRICS_ADDRESS:-127.0.0.1:${DEFAULT_META_PORT}}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

curl --fail --silent --show-error "http://${CLIENT_ADDRESS}/metrics" >"${TMP_DIR}/client.prom"
curl --fail --silent --show-error "http://${NODE_ADDRESS}/metrics" >"${TMP_DIR}/node.prom"
curl --fail --silent --show-error "http://${META_ADDRESS}/metrics" >"${TMP_DIR}/meta.prom"

require_metric() {
  local file="$1"
  local metric="$2"
  if ! grep -q "^# HELP ${metric} " "${file}"; then
    echo "missing metric contract ${metric} in ${file}" >&2
    exit 1
  fi
}

# 返回一个完整 Prometheus sample 的当前值。这里要求调用方给出完整的
# metric + 有界 labels，避免把多个时间序列误加在一起。
sample_value() {
  local file="$1"
  local sample="$2"
  awk -v sample="${sample}" '
    index($0, sample " ") == 1 { print $NF; found = 1; exit }
    END { if (!found) exit 1 }
  ' "${file}"
}

require_positive_sample() {
  local file="$1"
  local sample="$2"
  local value
  if ! value="$(sample_value "${file}" "${sample}")"; then
    echo "missing metric sample ${sample} in ${file}" >&2
    exit 1
  fi
  if ! awk -v value="${value}" 'BEGIN { exit !(value > 0) }'; then
    echo "expected ${sample} to be positive, found ${value}" >&2
    exit 1
  fi
}

require_not_less_samples() {
  local left_file="$1"
  local left_sample="$2"
  local right_file="$3"
  local right_sample="$4"
  local left_value right_value
  left_value="$(sample_value "${left_file}" "${left_sample}")"
  right_value="$(sample_value "${right_file}" "${right_sample}")"
  if ! awk -v left="${left_value}" -v right="${right_value}" 'BEGIN { exit !(left >= right) }'; then
    echo "metric ordering mismatch: ${left_sample}=${left_value}, ${right_sample}=${right_value}" >&2
    exit 1
  fi
}

for metric in \
  dms_client_operations_total \
  dms_client_node_session_events_total \
  dms_client_region_mapping_lookups_total \
  dms_client_region_mappings \
  dms_client_payload_transfers_total \
  dms_errors_total \
  dms_rpc_client_requests_total; do
  require_metric "${TMP_DIR}/client.prom" "${metric}"
done

# 薄 SDK 已删除跨请求 value cache；保留的是 Region FD/mmap 复用。
# 不能让已删除的业务指标以常零值继续存在，掩盖旧代码或旧候选包被部署。
if grep -q '^# HELP dms_client_cache_' "${TMP_DIR}/client.prom"; then
  echo "obsolete SDK value-cache metric family is still registered" >&2
  exit 1
fi

for metric in \
  dms_node_mailbox_depth \
  dms_node_arena_capacity_bytes \
  dms_node_arena_allocations_total \
  dms_node_current_cache_lookups_total \
  dms_node_current_cache_charged_bytes \
  dms_rpc_server_requests_total; do
  require_metric "${TMP_DIR}/node.prom" "${metric}"
done

for metric in \
  dms_meta_operations_total \
  dms_meta_commits_total \
  dms_meta_journal_appends_total \
  dms_meta_state_items \
  dms_rpc_server_requests_total; do
  require_metric "${TMP_DIR}/meta.prom" "${metric}"
done

# 旧 67 族门禁漏计了后来新增的 Node Current 两族；本轮又删除 SDK cache 两族。
# 当前业务/RPC/错误为 61 族，Trace 有 5 个预建族、1 个延迟产生的 export_batches。
# Trace 关闭/尚未导出时共 66，导出过后共 67；不能为凑数给不存在的导出补假数据。
# 多个进程共享的指标族按名称去重；实际路径的必需指标继续逐项校验。
family_count="$({
  grep '^# HELP dms_' "${TMP_DIR}/client.prom"
  grep '^# HELP dms_' "${TMP_DIR}/node.prom"
  grep '^# HELP dms_' "${TMP_DIR}/meta.prom"
} | awk '{print $3}' | sort -u | wc -l | tr -d ' ')"
require_metric "${TMP_DIR}/node.prom" dms_node_arena_quarantined_bytes
expected_family_count=66
if grep -q '^# HELP dms_trace_export_batches_total ' "${TMP_DIR}"/*.prom; then
  expected_family_count=67
fi
if [[ "${family_count}" != "${expected_family_count}" ]]; then
  echo "expected ${expected_family_count} unique DMS metric families, found ${family_count}" >&2
  exit 1
fi

if [[ "${TOPOLOGY}" == "single" ]]; then
  # metrics_host 会执行一次小对象 inline SET/GET，以及一次 96 KiB 的
  # SHM SET/GET。这里验证的不只是 HELP 合同，而是真实数据路径的结果。
  require_positive_sample "${TMP_DIR}/client.prom" \
    'dms_client_payload_bytes_total{direction="write",provider="shm"}'
  require_positive_sample "${TMP_DIR}/client.prom" \
    'dms_client_payload_bytes_total{direction="read",provider="shm"}'
  require_positive_sample "${TMP_DIR}/client.prom" \
    'dms_client_region_mapping_lookups_total{result="miss"}'
  require_positive_sample "${TMP_DIR}/node.prom" \
    'dms_node_shm_fd_grants_total{result="issued"}'
  require_positive_sample "${TMP_DIR}/node.prom" \
    'dms_node_shm_fd_grants_total{result="claimed"}'

  # metrics-host 只暴露它自己的 SDK Registry；Node 还会接收 sdk_api 等
  # 其他 Client 的调用。因此服务端完成数必须不少于这个宿主 Client 的
  # 调用数，不能错误地要求两个进程的局部 Registry 全局相等。
  require_not_less_samples \
    "${TMP_DIR}/node.prom" \
    'dms_rpc_server_requests_total{method="SetInline",result="ok",service="WorkerService"}' \
    "${TMP_DIR}/client.prom" \
    'dms_rpc_client_requests_total{method="SetInline",result="ok",service="WorkerService"}'
  require_not_less_samples \
    "${TMP_DIR}/node.prom" \
    'dms_rpc_server_requests_total{method="Set",result="ok",service="WorkerService"}' \
    "${TMP_DIR}/client.prom" \
    'dms_rpc_client_requests_total{method="Set",result="ok",service="WorkerService"}'
  require_not_less_samples \
    "${TMP_DIR}/meta.prom" \
    'dms_rpc_server_requests_total{method="CommitVersion",result="ok",service="MetadataService"}' \
    "${TMP_DIR}/node.prom" \
    'dms_rpc_client_requests_total{method="CommitVersion",result="ok",service="MetadataService"}'
fi

echo "Client/Node/Meta Metrics E2E passed: ${family_count} metric families and real RPC/SHM samples"

if [[ "${TOPOLOGY}" == "three-node" ]]; then
  N1_ADDRESS="${1:?usage: verify_metrics.sh --topology three-node N1_IP N2_IP N3_IP}"
  N2_ADDRESS="${2:?usage: verify_metrics.sh --topology three-node N1_IP N2_IP N3_IP}"
  N3_ADDRESS="${3:?usage: verify_metrics.sh --topology three-node N1_IP N2_IP N3_IP}"
  for address in "${N1_ADDRESS}" "${N2_ADDRESS}" "${N3_ADDRESS}"; do
    curl --noproxy '*' --fail --silent --show-error "http://${address}:19000/metrics" >/dev/null
  done
  curl --noproxy '*' --fail --silent --show-error "http://${N1_ADDRESS}:9090/api/v1/targets" \
    >"${TMP_DIR}/targets.json"
  python3 - "${TMP_DIR}/targets.json" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as stream:
    payload = json.load(stream)
active = payload["data"]["activeTargets"]
counts = {}
unhealthy = []
for target in active:
    job = target["labels"].get("job", "")
    counts[job] = counts.get(job, 0) + 1
    if target.get("health") != "up":
        unhealthy.append((job, target.get("scrapeUrl"), target.get("lastError")))
expected = {"dms-node": 3, "dms-meta": 1, "dms-client": 1}
if counts != expected or unhealthy:
    raise SystemExit(f"unexpected Prometheus targets: counts={counts} unhealthy={unhealthy}")
print(f"Prometheus targets passed: {counts}")
PY
fi
