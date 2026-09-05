#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
if [[ "${DMS_DEV_ENV:-}" == "1" && -f "${SCRIPTS_DIR}/_common.sh" ]]; then
  source "${SCRIPTS_DIR}/_common.sh"
else
  DMS_SOURCE_DIR="$(cd "${SCRIPTS_DIR}/.." && pwd -P)"
  DMS_LOG_DIR="${DMS_SOURCE_DIR}/log"
fi

TOPOLOGY="single"
if [[ "${1:-}" == "--topology" ]]; then
  TOPOLOGY="${2:?missing topology after --topology}"
  shift 2
fi

LOKI_ADDRESS="${DMS_LOKI_ADDRESS:-127.0.0.1:3100}"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT

python3 - "${DMS_LOG_DIR}/dms-node.log" "${DMS_LOG_DIR}/dms-meta.log" <<'PY'
import json
import pathlib
import sys

expected = [(pathlib.Path(sys.argv[1]), "dms-node"), (pathlib.Path(sys.argv[2]), "dms-meta")]
for path, service in expected:
    if not path.is_file():
        raise SystemExit(f"missing process log: {path}")
    records = []
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            records.append(json.loads(line))
        except json.JSONDecodeError as error:
            raise SystemExit(f"non-JSON record in {path}: {error}: {line[:160]}")
    if not any(record.get("service_name") == service for record in records):
        raise SystemExit(f"{path} has no service_name={service} record")
    if not any(record.get("event") == f"{service.removeprefix('dms-')}.process.ready" for record in records):
        raise SystemExit(f"{path} has no ready event")
    required = {"ts", "level", "msg", "service_name", "instance", "event"}
    missing = required.difference(records[0])
    if missing:
        raise SystemExit(f"{path} first record misses required fields: {sorted(missing)}")
print("local JSON logs passed")
PY

ready=false
for _ in $(seq 1 30); do
  if curl --fail --silent "http://${LOKI_ADDRESS}/ready" >/dev/null; then
    ready=true
    break
  fi
  sleep 1
done
if [[ "${ready}" != "true" ]]; then
  echo "Loki is not ready at ${LOKI_ADDRESS}" >&2
  exit 1
fi

query='{service_name=~"dms-(node|meta)"}'
found=false
for _ in $(seq 1 30); do
  curl --fail --silent --show-error -G \
    --data-urlencode "query=${query}" \
    --data-urlencode 'limit=200' \
    "http://${LOKI_ADDRESS}/loki/api/v1/query_range" >"${TMP_DIR}/query.json"
  if python3 - "${TMP_DIR}/query.json" "${TOPOLOGY}" "$@" <<'PY'
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
streams = payload.get("data", {}).get("result", [])
services = {item.get("stream", {}).get("service_name") for item in streams}
if not {"dms-node", "dms-meta"}.issubset(services):
    raise SystemExit(1)
if sys.argv[2] == "three-node":
    expected = set(sys.argv[3:])
    nodes = {
        item.get("stream", {}).get("instance")
        for item in streams
        if item.get("stream", {}).get("service_name") == "dms-node"
    }
    if not expected.issubset(nodes):
        raise SystemExit(1)
print(f"Loki query passed: services={sorted(services)}")
PY
  then
    found=true
    break
  fi
  sleep 1
done

if [[ "${found}" != "true" ]]; then
  echo "Loki did not return the expected DMS log streams" >&2
  cat "${TMP_DIR}/query.json" >&2 || true
  exit 1
fi

GRAFANA_ADDRESS="${DMS_GRAFANA_ADDRESS:-127.0.0.1:3000}"
GRAFANA_USER="${DMS_GRAFANA_USER:-admin}"
GRAFANA_PASSWORD="${DMS_GRAFANA_PASSWORD:-dms-dev}"
curl --fail --silent --show-error \
  --user "${GRAFANA_USER}:${GRAFANA_PASSWORD}" \
  "http://${GRAFANA_ADDRESS}/api/datasources/uid/dms-loki" >"${TMP_DIR}/datasource.json"
curl --fail --silent --show-error \
  --user "${GRAFANA_USER}:${GRAFANA_PASSWORD}" \
  "http://${GRAFANA_ADDRESS}/api/dashboards/uid/dms-overview" >"${TMP_DIR}/dashboard.json"
python3 - "${TMP_DIR}/datasource.json" "${TMP_DIR}/dashboard.json" <<'PY'
import json
import sys

datasource = json.load(open(sys.argv[1], encoding="utf-8"))
dashboard = json.load(open(sys.argv[2], encoding="utf-8"))["dashboard"]
if datasource.get("type") != "loki" or datasource.get("url") != "http://loki:3100":
    raise SystemExit(f"unexpected Loki datasource: {datasource}")
panels = {(panel.get("title"), panel.get("type")) for panel in dashboard.get("panels", [])}
if ("DMS process logs", "logs") not in panels:
    raise SystemExit(f"DMS process logs panel is missing: {sorted(panels)}")
print("Grafana provisioning passed: Loki datasource and DMS process logs panel")
PY
