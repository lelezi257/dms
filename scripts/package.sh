#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

ARCH="$(uname -m)"
WORKSPACE_VERSION="$(
  awk '
    /^\[workspace\.package\]$/ { in_workspace_package = 1; next }
    /^\[/ { in_workspace_package = 0 }
    in_workspace_package && $1 == "version" {
      gsub(/"/, "", $3)
      print $3
      exit
    }
  ' "${DMS_SOURCE_DIR}/Cargo.toml"
)"
if [[ -z "${WORKSPACE_VERSION}" ]]; then
  echo "无法从 Cargo.toml 读取 workspace.package.version" >&2
  exit 2
fi
if [[ -n "${DMS_SERVER_VERSION:-}" && "${DMS_SERVER_VERSION}" != "${WORKSPACE_VERSION}" ]]; then
  echo "DMS_SERVER_VERSION=${DMS_SERVER_VERSION} 与 Cargo workspace 版本 ${WORKSPACE_VERSION} 不一致，拒绝构包" >&2
  exit 2
fi
VERSION="${WORKSPACE_VERSION}"
PACKAGE_NAME="dms-server-${VERSION}-linux-${ARCH}"
OUTPUT_ROOT="$(cd "${1:-${DMS_SOURCE_DIR}/artifacts/s4-candidate}" 2>/dev/null || true)"
if [[ -z "${OUTPUT_ROOT}" ]]; then
  mkdir -p "${1:-${DMS_SOURCE_DIR}/artifacts/s4-candidate}"
  OUTPUT_ROOT="$(cd "${1:-${DMS_SOURCE_DIR}/artifacts/s4-candidate}" && pwd -P)"
fi
BUILD_ID="${DMS_PACKAGE_BUILD_ID:-$(date -u +%Y%m%dT%H%M%SZ)-$$}"
if [[ ! "${BUILD_ID}" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ || "${BUILD_ID}" == *..* ]]; then
  echo "非法 DMS_PACKAGE_BUILD_ID=${BUILD_ID}；只允许字母数字、点、下划线、短横线，且不能包含 '..'" >&2
  exit 2
fi
OUTPUT_DIR="${OUTPUT_ROOT}/${PACKAGE_NAME}-${BUILD_ID}"
STAGE_DIR="${OUTPUT_DIR}/${PACKAGE_NAME}"
ARCHIVE="${OUTPUT_DIR}/${PACKAGE_NAME}.tar.gz"

declare -A artifacts=(
  [dms-node]="${CARGO_TARGET_DIR}/release/dms-node"
  [dms-meta]="${CARGO_TARGET_DIR}/release/dms-meta"
  [dms-health]="${CARGO_TARGET_DIR}/release/dms-health"
  [dms-metrics-host]="${CARGO_TARGET_DIR}/release/examples/metrics_host"
  [sdk-kv]="${CARGO_TARGET_DIR}/release/examples/sdk_kv"
)

for name in "${!artifacts[@]}"; do
  if [[ ! -x "${artifacts[${name}]}" ]]; then
    echo "缺少 ${name}，请先执行 ./scripts/build.sh" >&2
    exit 2
  fi
done

if [[ -e "${OUTPUT_DIR}" ]]; then
  echo "候选包输出目录已存在，拒绝覆盖：${OUTPUT_DIR}" >&2
  exit 2
fi

mkdir -p \
  "${STAGE_DIR}/bin" \
  "${STAGE_DIR}/config" \
  "${STAGE_DIR}/scripts" \
  "${STAGE_DIR}/infra/observability" \
  "${STAGE_DIR}/docs"

for name in "${!artifacts[@]}"; do
  install -m 0755 "${artifacts[${name}]}" "${STAGE_DIR}/bin/${name}"
done

install -m 0644 "${SCRIPTS_DIR}/cluster.env.example" "${STAGE_DIR}/config/dms.env.example"
install -m 0755 "${SCRIPTS_DIR}/cluster.sh" "${STAGE_DIR}/scripts/cluster.sh"
install -m 0755 "${SCRIPTS_DIR}/metrics-targets.sh" "${STAGE_DIR}/scripts/metrics-targets.sh"
install -m 0755 "${SCRIPTS_DIR}/observability.sh" "${STAGE_DIR}/scripts/observability.sh"
install -m 0755 "${SCRIPTS_DIR}/verify_metrics.sh" "${STAGE_DIR}/scripts/verify_metrics.sh"
install -m 0755 "${SCRIPTS_DIR}/verify_logs.sh" "${STAGE_DIR}/scripts/verify_logs.sh"
install -m 0755 "${SCRIPTS_DIR}/verify_tracing.sh" "${STAGE_DIR}/scripts/verify_tracing.sh"

install -m 0644 "${DMS_SOURCE_DIR}/infra/observability/compose.yaml" "${STAGE_DIR}/infra/observability/compose.yaml"
while IFS= read -r path; do
  mkdir -p "${STAGE_DIR}/$(dirname "${path}")"
  install -m 0644 "${DMS_SOURCE_DIR}/${path}" "${STAGE_DIR}/${path}"
done <<'FILES'
infra/observability/alloy/config.alloy
infra/observability/grafana/dashboards/dms-overview.json
infra/observability/grafana/provisioning/dashboards/dms.yaml
infra/observability/grafana/provisioning/datasources/dms-loki.yaml
infra/observability/grafana/provisioning/datasources/dms-prometheus.yaml
infra/observability/grafana/provisioning/datasources/dms-tempo.yaml
infra/observability/loki/loki.yaml
infra/observability/prometheus/file_sd/targets.json
infra/observability/prometheus/prometheus.yml
infra/observability/tempo/Dockerfile
infra/observability/tempo/tempo.yaml
FILES

# 运行包不携带要求源码树的开发教程；避免用户点到缺失的 SDK/AGENTS 链接。
install -m 0644 "${DMS_SOURCE_DIR}/docs/release-installation.md" "${STAGE_DIR}/docs/release-installation.md"
install -m 0755 "${SCRIPTS_DIR}/release/candidate_registry.py" "${STAGE_DIR}/scripts/candidate_registry.py"

for legal in LICENSE NOTICE CHANGELOG.md; do
  if [[ -f "${DMS_SOURCE_DIR}/${legal}" ]]; then
    install -m 0644 "${DMS_SOURCE_DIR}/${legal}" "${STAGE_DIR}/${legal}"
  else
    echo "候选包未包含 ${legal}：源文件尚未提供，S4 法务/发布元信息仍需主流程确认" >&2
  fi
done

THIRD_PARTY_DIR="${DMS_THIRD_PARTY_DIR:-${DMS_SOURCE_DIR}/THIRD-PARTY-LICENSES}"
if [[ -d "${THIRD_PARTY_DIR}" ]]; then
  while IFS= read -r path; do
    mkdir -p "${STAGE_DIR}/THIRD-PARTY-LICENSES/$(dirname "${path}")"
    install -m 0644 "${THIRD_PARTY_DIR}/${path}" "${STAGE_DIR}/THIRD-PARTY-LICENSES/${path}"
  done < <(cd "${THIRD_PARTY_DIR}" && find . -type f -printf '%P\n' | LC_ALL=C sort)
else
  echo "候选包未包含 THIRD-PARTY-LICENSES/：第三方许可清单由 S4 主流程生成后通过 DMS_THIRD_PARTY_DIR 指定" >&2
fi

(
  cd "${STAGE_DIR}"
  BUILT_AT_UTC="$(date -u -d "@${SOURCE_DATE_EPOCH:-$(date +%s)}" +%Y-%m-%dT%H:%M:%SZ)"
  {
    echo "name=${PACKAGE_NAME}"
    echo "version=${VERSION}"
    echo "arch=${ARCH}"
    echo "built_at_utc=${BUILT_AT_UTC}"
    echo "source=Linux Cargo release artifacts"
    echo "legal_status=候选包；LICENSE/NOTICE/CHANGELOG 以源树现有文件为准，缺失不在脚本中编造主体"
  } >PACKAGE-METADATA
  find . -type f ! -name SHA256SUMS ! -name MANIFEST.txt -printf '%P\n' | LC_ALL=C sort >MANIFEST.txt
  xargs -r sha256sum <MANIFEST.txt >SHA256SUMS
)

tar --sort=name \
  --mtime="@${SOURCE_DATE_EPOCH:-0}" \
  --owner=0 \
  --group=0 \
  --numeric-owner \
  -C "${OUTPUT_DIR}" \
  -cf - "${PACKAGE_NAME}" \
  | gzip -n >"${ARCHIVE}"
(
  cd "${OUTPUT_DIR}"
  sha256sum "$(basename "${ARCHIVE}")" >"$(basename "${ARCHIVE}").sha256"
)
echo "运行包已生成：${ARCHIVE}"
echo "归档校验：${ARCHIVE}.sha256"
echo "解压目录：${PACKAGE_NAME}/"
