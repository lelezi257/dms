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
TEMPO_ADDRESS="${DMS_TEMPO_ADDRESS:-127.0.0.1:3200}"
LOKI_ADDRESS="${DMS_LOKI_ADDRESS:-127.0.0.1:3100}"
GRAFANA_ADDRESS="${DMS_GRAFANA_ADDRESS:-127.0.0.1:3000}"
PROMETHEUS_ADDRESS="${DMS_PROMETHEUS_ADDRESS:-127.0.0.1:9090}"
GRAFANA_USER="${DMS_GRAFANA_USER:-admin}"
GRAFANA_PASSWORD="${DMS_GRAFANA_PASSWORD:-dms-dev}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

wait_http_ready() {
  local name="$1"
  local url="$2"
  local attempts="${3:-60}"

  for ((attempt = 1; attempt <= attempts; attempt++)); do
    if curl --fail --silent --show-error "${url}" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.5
  done

  echo "${name} did not become ready: ${url}" >&2
  return 1
}

wait_for_exported_span() {
  local name="$1"
  local address="$2"
  local output="$3"

  for ((attempt = 1; attempt <= 60; attempt++)); do
    if curl --fail --silent --show-error "http://${address}/metrics" >"${output}" 2>/dev/null \
        && awk '$1 == "dms_trace_exported_spans_total" && $2 + 0 > 0 { found = 1 } END { exit !found }' "${output}"; then
      return 0
    fi
    sleep 0.5
  done

  echo "${name} exported no spans before timeout: http://${address}/metrics" >&2
  return 1
}

# Docker Compose 返回只表示容器已经启动，不表示 Tempo 已完成初始化。
# 把正常的启动等待放在验证入口，避免手册要求用户猜一个 sleep 时间。
wait_http_ready "Tempo" "http://${TEMPO_ADDRESS}/ready"
wait_for_exported_span "Client" "${CLIENT_ADDRESS}" "${TMP_DIR}/client.prom"
wait_for_exported_span "Node" "${NODE_ADDRESS}" "${TMP_DIR}/node.prom"
wait_for_exported_span "Meta" "${META_ADDRESS}" "${TMP_DIR}/meta.prom"

if [[ "${TOPOLOGY}" == "single" ]]; then
  # Let all operation spans created during startup/metrics-host initialization
  # pass one exporter batch boundary. Then prove that healthy 5-second
  # Heartbeats still increment Metrics without increasing exported spans.
  sleep 6
  curl --fail --silent --show-error "http://${NODE_ADDRESS}/metrics" >"${TMP_DIR}/node-periodic-before.prom"
  curl --fail --silent --show-error "http://${META_ADDRESS}/metrics" >"${TMP_DIR}/meta-periodic-before.prom"
  sleep 6
  curl --fail --silent --show-error "http://${NODE_ADDRESS}/metrics" >"${TMP_DIR}/node-periodic-after.prom"
  curl --fail --silent --show-error "http://${META_ADDRESS}/metrics" >"${TMP_DIR}/meta-periodic-after.prom"
fi

# The metrics host proves the library-level dms.client.* tree. Run the public
# sdk-kv example as a second consumer so its user-facing root names are also an
# executable contract instead of a documentation-only convention.
SDK_KV_BIN="${DMS_SDK_KV_BIN:-}"
for candidate in \
  "${DMS_SOURCE_DIR}/bin/sdk-kv" \
  "${CARGO_TARGET_DIR:-}/release/examples/sdk_kv" \
  "${DMS_SOURCE_DIR}/artifacts/dms-linux-$(uname -m)/bin/sdk-kv"; do
  if [[ -z "${SDK_KV_BIN}" && -x "${candidate}" ]]; then
    SDK_KV_BIN="${candidate}"
  fi
done
if [[ ! -x "${SDK_KV_BIN}" ]]; then
  echo "sdk-kv binary not found; set DMS_SDK_KV_BIN or run ./scripts/build.sh" >&2
  exit 1
fi

SDK_ENDPOINT="${DMS_ENDPOINT:-${DMS_CLIENT_ENDPOINT:-unix://${DMS_WORKER_UDS:-/tmp/dms-local-dev/run/dms-worker.sock}}}"
SDK_KEY="tracing/e2e-operation-name-$$"
SDK_SET_OUTPUT="$(
  DMS_ENDPOINT="${SDK_ENDPOINT}" \
  DMS_CLIENT_INSTANCE="tracing-e2e-set" \
  DMS_TRACING_ENABLED="${DMS_TRACING_ENABLED:-false}" \
  DMS_TRACING_OTLP_ENDPOINT="${DMS_TRACING_OTLP_ENDPOINT:-http://127.0.0.1:4317}" \
  DMS_TRACING_SAMPLE_RATIO="${DMS_TRACING_SAMPLE_RATIO:-0.01}" \
  "${SDK_KV_BIN}" set "${SDK_KEY}" "operation-aware"
)"
SDK_GET_OUTPUT="$(
  DMS_ENDPOINT="${SDK_ENDPOINT}" \
  DMS_CLIENT_INSTANCE="tracing-e2e-get" \
  DMS_TRACING_ENABLED="${DMS_TRACING_ENABLED:-false}" \
  DMS_TRACING_OTLP_ENDPOINT="${DMS_TRACING_OTLP_ENDPOINT:-http://127.0.0.1:4317}" \
  DMS_TRACING_SAMPLE_RATIO="${DMS_TRACING_SAMPLE_RATIO:-0.01}" \
  "${SDK_KV_BIN}" get "${SDK_KEY}" "operation-aware"
)"
DMS_SDK_SET_TRACE_ID="$(sed -n 's/^trace_id=//p' <<<"${SDK_SET_OUTPUT}" | tail -1)"
DMS_SDK_GET_TRACE_ID="$(sed -n 's/^trace_id=//p' <<<"${SDK_GET_OUTPUT}" | tail -1)"
if [[ ! "${DMS_SDK_SET_TRACE_ID}" =~ ^[0-9a-f]{32}$ \
    || ! "${DMS_SDK_GET_TRACE_ID}" =~ ^[0-9a-f]{32}$ ]]; then
  echo "sdk-kv did not emit valid SET/GET trace IDs" >&2
  exit 1
fi
export DMS_SDK_SET_TRACE_ID DMS_SDK_GET_TRACE_ID

python3 - "${TMP_DIR}" "${TEMPO_ADDRESS}" "${LOKI_ADDRESS}" \
  "${GRAFANA_ADDRESS}" "${PROMETHEUS_ADDRESS}" "${GRAFANA_USER}" "${GRAFANA_PASSWORD}" "${TOPOLOGY}" <<'PY'
import json
import base64
import os
import re
import sys
import time
import urllib.parse
import urllib.request
from pathlib import Path

tmp = Path(sys.argv[1])
tempo, loki, grafana, prometheus, grafana_user, grafana_password, topology = sys.argv[2:]

def get(url, timeout=5, basic_auth=None):
    request = urllib.request.Request(url)
    if basic_auth:
        encoded = base64.b64encode(basic_auth.encode("utf-8")).decode("ascii")
        request.add_header("Authorization", f"Basic {encoded}")
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read().decode("utf-8")

def metric_value(path, name):
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith(name + " "):
            return float(line.split()[1])
    raise SystemExit(f"missing metric {name} in {path}")

def labeled_metric_value(path, name, expected_labels):
    prefix = name + "{"
    for line in path.read_text(encoding="utf-8").splitlines():
        if not line.startswith(prefix):
            continue
        labels_text, value = line.split("} ", 1)
        labels = dict(re.findall(r'(\w+)="([^"]*)"', labels_text))
        if all(labels.get(key) == expected for key, expected in expected_labels.items()):
            return float(value.split()[0])
    raise SystemExit(f"missing metric {name}{expected_labels} in {path}")

for name in ("client", "node", "meta"):
    exported = metric_value(tmp / f"{name}.prom", "dms_trace_exported_spans_total")
    if exported <= 0:
        raise SystemExit(f"{name} exported no spans: {exported}")

text = (tmp / "client.prom").read_text(encoding="utf-8")
trace_ids = list(dict.fromkeys(re.findall(
    r'dms_client_operation_duration_seconds_bucket\{operation="set"[^\n]*# \{trace_id="([0-9a-f]{32})"\}',
    text,
)))
if not trace_ids:
    raise SystemExit("SET histogram has no trace exemplar")

selected = None
for _ in range(30):
    for trace_id in trace_ids:
        try:
            trace = json.loads(get(f"http://{tempo}/api/traces/{trace_id}"))
        except Exception:
            continue
        rows = []
        for batch in trace.get("batches", []):
            attrs = {
                item["key"]: next(iter(item.get("value", {}).values()), "")
                for item in batch.get("resource", {}).get("attributes", [])
            }
            service = attrs.get("service.name", "")
            for scope in batch.get("scopeSpans", []):
                for span in scope.get("spans", []):
                    rows.append({"service": service, **span})
        names = {row.get("name") for row in rows}
        services = {row.get("service") for row in rows}
        if {"dms-client", "dms-node", "dms-meta"} <= services \
                and "dms.client.set" in names \
                and "dms.payload.upload" in names:
            selected = (trace_id, rows)
            break
    if selected:
        break
    time.sleep(0.25)
if not selected:
    raise SystemExit("no SET exemplar resolved to a Client→Node→Meta trace with payload span")

trace_id, rows = selected
if not (5 <= len(rows) <= 100):
    raise SystemExit(f"unexpected span count {len(rows)}; possible missing spans or export recursion")
roots = [row for row in rows if row.get("name") == "dms.client.set"]
if len(roots) != 1 or roots[0].get("parentSpanId"):
    raise SystemExit("dms.client.set is not the unique root span")
root_id = roots[0]["spanId"]
if not any(row.get("service") == "dms-node"
           and row.get("name", "").startswith("dms.grpc.worker.")
           and row.get("parentSpanId") == root_id for row in rows):
    raise SystemExit("operation-aware Node gRPC span is not a direct child of the Client root")
node_ids = {row.get("spanId") for row in rows if row.get("service") == "dms-node"}
if not any(row.get("service") == "dms-meta"
           and row.get("name") == "dms.grpc.meta.commit_version"
           and row.get("parentSpanId") in node_ids for row in rows):
    raise SystemExit("Meta commit_version span is not parented by a Node span")

generic_names = {
    "dms.grpc.server",
    "dms.node.command",
    "dms.meta.command",
    "dms.payload.transfer",
    "dms.sdk_kv",
}
unexpected_generic_names = names & generic_names
if unexpected_generic_names:
    raise SystemExit(
        f"known SET path still exports generic span names: {sorted(unexpected_generic_names)}"
    )

query = urllib.parse.urlencode({
    "query": f'{{job="dms"}} |= "{trace_id}"',
    "limit": "100",
})
for _ in range(20):
    payload = json.loads(get(f"http://{loki}/loki/api/v1/query_range?{query}"))
    log_services = {
        item.get("stream", {}).get("service_name")
        for item in payload.get("data", {}).get("result", [])
    }
    if {"dms-node", "dms-meta"} <= log_services:
        break
    time.sleep(0.25)
else:
    raise SystemExit(f"Loki has no correlated Node+Meta logs for trace {trace_id}")

datasources = json.loads(get(
    f"http://{grafana}/api/datasources",
    basic_auth=f"{grafana_user}:{grafana_password}",
))
by_uid = {item["uid"]: item for item in datasources}
if by_uid.get("dms-tempo", {}).get("url") != "http://tempo:3200":
    raise SystemExit("Grafana Tempo datasource is not provisioned")
prom_json = by_uid.get("dms-prometheus", {}).get("jsonData", {})
if not any(item.get("datasourceUid") == "dms-tempo"
           for item in prom_json.get("exemplarTraceIdDestinations", [])):
    raise SystemExit("Prometheus exemplar → Tempo link is missing")
loki_json = by_uid.get("dms-loki", {}).get("jsonData", {})
if not any(item.get("datasourceUid") == "dms-tempo"
           for item in loki_json.get("derivedFields", [])):
    raise SystemExit("Loki trace_id → Tempo link is missing")

query = urllib.parse.urlencode({
    "query": 'dms_client_operation_duration_seconds_bucket{operation="set"}',
    "start": str(int(time.time()) - 3600),
    "end": str(int(time.time()) + 60),
})
exemplars = json.loads(get(f"http://{prometheus}/api/v1/query_exemplars?{query}"))
if exemplars.get("status") != "success":
    raise SystemExit("Prometheus exemplar query failed")

summary = {
    "trace_id": trace_id,
    "span_count": len(rows),
    "services": sorted({row["service"] for row in rows}),
    "span_names": sorted({row["name"] for row in rows}),
    "log_services": sorted(log_services),
}

sdk_roots = {
    os.environ["DMS_SDK_SET_TRACE_ID"]: "dms.sdk_kv.set",
    os.environ["DMS_SDK_GET_TRACE_ID"]: "dms.sdk_kv.get",
}
summary["sdk_roots"] = {}
for sdk_trace_id, expected_root in sdk_roots.items():
    sdk_rows = None
    for _ in range(30):
        try:
            sdk_trace = json.loads(get(f"http://{tempo}/api/traces/{sdk_trace_id}"))
        except Exception:
            time.sleep(0.25)
            continue
        candidate_rows = []
        for batch in sdk_trace.get("batches", []):
            attrs = {
                item["key"]: next(iter(item.get("value", {}).values()), "")
                for item in batch.get("resource", {}).get("attributes", [])
            }
            service = attrs.get("service.name", "")
            for scope in batch.get("scopeSpans", []):
                for span in scope.get("spans", []):
                    candidate_rows.append({"service": service, **span})
        if candidate_rows:
            sdk_rows = candidate_rows
            break
        time.sleep(0.25)
    if sdk_rows is None:
        raise SystemExit(f"sdk-kv trace not found in Tempo: {sdk_trace_id}")
    roots = [row for row in sdk_rows if not row.get("parentSpanId")]
    if len(roots) != 1 or roots[0].get("name") != expected_root:
        raise SystemExit(
            f"sdk-kv trace root mismatch: expected {expected_root}, "
            f"got {[row.get('name') for row in roots]}"
        )
    summary["sdk_roots"][sdk_trace_id] = expected_root

# A cross-node GET probe can pass its printed trace ID through this optional
# environment variable. The same verifier then proves Node→Node propagation,
# rather than merely checking that all three Node exporters are alive.
peer_trace_id = os.environ.get("DMS_PEER_TRACE_ID")
if peer_trace_id:
    peer = json.loads(get(f"http://{tempo}/api/traces/{peer_trace_id}"))
    peer_services = set()
    node_instances = set()
    peer_names = set()
    for batch in peer.get("batches", []):
        attrs = {
            item["key"]: next(iter(item.get("value", {}).values()), "")
            for item in batch.get("resource", {}).get("attributes", [])
        }
        service = attrs.get("service.name", "")
        peer_services.add(service)
        if service == "dms-node":
            node_instances.add(attrs.get("service.instance.id", ""))
        for scope in batch.get("scopeSpans", []):
            peer_names.update(span.get("name") for span in scope.get("spans", []))
    if not {"dms-client", "dms-node", "dms-meta"} <= peer_services:
        raise SystemExit(f"peer trace misses process roles: {sorted(peer_services)}")
    if len(node_instances) < 2:
        raise SystemExit(f"peer trace did not cross two Node instances: {sorted(node_instances)}")
    if not {"dms.sdk_kv.get", "dms.client.get", "dms.payload.download"} <= peer_names:
        raise SystemExit(f"peer trace misses expected GET/payload spans: {sorted(peer_names)}")
    summary["peer_trace_id"] = peer_trace_id
    summary["peer_node_instances"] = sorted(node_instances)

if topology == "single":
    node_before = tmp / "node-periodic-before.prom"
    node_after = tmp / "node-periodic-after.prom"
    meta_before = tmp / "meta-periodic-before.prom"
    meta_after = tmp / "meta-periodic-after.prom"
    heartbeat_labels = {
        "service": "MetadataService",
        "method": "Heartbeat",
        "result": "ok",
    }
    heartbeats_before = labeled_metric_value(
        node_before, "dms_rpc_client_requests_total", heartbeat_labels
    )
    heartbeats_after = labeled_metric_value(
        node_after, "dms_rpc_client_requests_total", heartbeat_labels
    )
    if heartbeats_after <= heartbeats_before:
        raise SystemExit(
            "healthy Meta Heartbeat did not continue incrementing RPC Metrics"
        )
    for role, before_path, after_path in (
        ("node", node_before, node_after),
        ("meta", meta_before, meta_after),
    ):
        before = metric_value(before_path, "dms_trace_exported_spans_total")
        after = metric_value(after_path, "dms_trace_exported_spans_total")
        if after != before:
            raise SystemExit(
                f"{role} exported spans increased during heartbeat-only window: {before} -> {after}"
            )
    summary["periodic_heartbeat"] = {
        "metrics_before": heartbeats_before,
        "metrics_after": heartbeats_after,
        "trace_exports_unchanged": True,
    }

(tmp / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
print(json.dumps(summary, ensure_ascii=False))
PY

if [[ "${TOPOLOGY}" == "three-node" ]]; then
  N1_ADDRESS="${1:?usage: verify_tracing.sh --topology three-node N1_IP N2_IP N3_IP}"
  N2_ADDRESS="${2:?usage: verify_tracing.sh --topology three-node N1_IP N2_IP N3_IP}"
  N3_ADDRESS="${3:?usage: verify_tracing.sh --topology three-node N1_IP N2_IP N3_IP}"
  for address in "${N1_ADDRESS}" "${N2_ADDRESS}" "${N3_ADDRESS}"; do
    curl --noproxy '*' --fail --silent --show-error \
      "http://${address}:19000/metrics" \
      | grep -q '^dms_trace_exported_spans_total [1-9]'
  done
fi

echo "Tracing E2E passed: exemplar → distributed trace → correlated logs → Grafana links"
