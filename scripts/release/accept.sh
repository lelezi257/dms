#!/usr/bin/env bash
# Run the S4 SDK + server candidate acceptance in an isolated Ubuntu container.
# This script prepares inputs and evidence, then runs isolated_consumer.py
# without mounting the DMS source tree into the container.
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
usage: scripts/release/accept.sh --sdk-crate SDK.crate --server-archive SERVER.tar.gz --output-dir OUT [options]

Required:
  --sdk-crate PATH          dms-client .crate candidate
  --server-archive PATH     dms-server-*.tar.gz candidate
  --output-dir PATH         new evidence/output directory; must not exist

Options:
  --otlp URL                OTLP endpoint passed to isolated_consumer.py
                            (default: http://127.0.0.1:24317)
  --port-base PORT          port base for isolated_consumer.py (default: 26000)
  --keep-running           leave the acceptance container running after base E2E
                            for separate observability checks
  --vendor-dir PATH         use an existing cargo vendor directory instead of
                            generating OUT/third-party-vendor
  -h, --help                show this help

Environment:
  DMS_ACCEPTANCE_IMAGE      image tag to use/build (default: dms-s4-buildenv:20260905)
  DMS_ACCEPTANCE_PROJECT    safe prefix for generated container name (default: dms-s4-accept)
  DMS_ACCEPTANCE_KEEP_WAIT_SECONDS
                            max seconds to wait for keep-running base E2E
                            (default: 1800)
  DMS_RUST_SYSROOT          Rust toolchain sysroot to mount read-only at /opt/rust
EOF
}

die() {
  echo "accept.sh: $*" >&2
  exit 2
}

require_safe_name() {
  local label="$1" value="$2"
  if [[ ! "${value}" =~ ^[A-Za-z0-9][A-Za-z0-9_.-]*$ || "${value}" == *..* ]]; then
    die "${label} 只能包含字母数字、点、下划线、短横线，且不能包含 '..': ${value}"
  fi
}

require_value() {
  local flag="$1" value="${2:-}"
  [[ -n "${value}" ]] || die "${flag} requires a value"
}

SDK_CRATE=""
SERVER_ARCHIVE=""
OUTPUT_DIR=""
OTLP_ENDPOINT="http://127.0.0.1:24317"
PORT_BASE="26000"
KEEP_RUNNING="false"
EXISTING_VENDOR_DIR=""

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --sdk-crate)
      require_value "$1" "${2:-}"
      SDK_CRATE="${2:-}"
      shift 2
      ;;
    --server-archive)
      require_value "$1" "${2:-}"
      SERVER_ARCHIVE="${2:-}"
      shift 2
      ;;
    --output-dir)
      require_value "$1" "${2:-}"
      OUTPUT_DIR="${2:-}"
      shift 2
      ;;
    --otlp)
      require_value "$1" "${2:-}"
      OTLP_ENDPOINT="${2:-}"
      shift 2
      ;;
    --port-base)
      require_value "$1" "${2:-}"
      PORT_BASE="${2:-}"
      shift 2
      ;;
    --keep-running)
      KEEP_RUNNING="true"
      shift
      ;;
    --vendor-dir)
      require_value "$1" "${2:-}"
      EXISTING_VENDOR_DIR="${2:-}"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[[ "$(uname -s)" == "Linux" ]] || die "请在 Linux dms-dev 内运行；macOS 只用于编辑"
[[ -n "${SDK_CRATE}" ]] || die "missing --sdk-crate"
[[ -n "${SERVER_ARCHIVE}" ]] || die "missing --server-archive"
[[ -n "${OUTPUT_DIR}" ]] || die "missing --output-dir"
[[ "${PORT_BASE}" =~ ^[0-9]+$ ]] || die "--port-base must be numeric: ${PORT_BASE}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SOURCE_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd -P)"
SDK_CRATE="$(readlink -f "${SDK_CRATE}")"
SERVER_ARCHIVE="$(readlink -f "${SERVER_ARCHIVE}")"
OUTPUT_PARENT="$(dirname "${OUTPUT_DIR}")"
mkdir -p "${OUTPUT_PARENT}"
OUTPUT_PARENT="$(cd "${OUTPUT_PARENT}" && pwd -P)"
OUTPUT_DIR="${OUTPUT_PARENT}/$(basename "${OUTPUT_DIR}")"

[[ -f "${SDK_CRATE}" ]] || die "SDK crate not found: ${SDK_CRATE}"
[[ -f "${SERVER_ARCHIVE}" ]] || die "server archive not found: ${SERVER_ARCHIVE}"
[[ ! -e "${OUTPUT_DIR}" ]] || die "output dir already exists; refusing to overwrite: ${OUTPUT_DIR}"

PROJECT_PREFIX="${DMS_ACCEPTANCE_PROJECT:-dms-s4-accept}"
require_safe_name "DMS_ACCEPTANCE_PROJECT" "${PROJECT_PREFIX}"
CONTAINER_NAME="${PROJECT_PREFIX}-$(date -u +%Y%m%dT%H%M%SZ)-$$"
require_safe_name "container name" "${CONTAINER_NAME}"
KEEP_WAIT_SECONDS="${DMS_ACCEPTANCE_KEEP_WAIT_SECONDS:-1800}"
[[ "${KEEP_WAIT_SECONDS}" =~ ^[0-9]+$ ]] || die "DMS_ACCEPTANCE_KEEP_WAIT_SECONDS must be numeric: ${KEEP_WAIT_SECONDS}"

IMAGE="${DMS_ACCEPTANCE_IMAGE:-dms-s4-buildenv:20260905}"
RUST_SYSROOT_DIR="${DMS_RUST_SYSROOT:-$(rustc --print sysroot)}"
RUST_SYSROOT_DIR="$(readlink -f "${RUST_SYSROOT_DIR}")"
[[ -d "${RUST_SYSROOT_DIR}" ]] || die "Rust sysroot not found: ${RUST_SYSROOT_DIR}"
[[ -x "${RUST_SYSROOT_DIR}/bin/cargo" ]] || die "cargo not found in Rust sysroot: ${RUST_SYSROOT_DIR}/bin/cargo"
[[ -x "${RUST_SYSROOT_DIR}/bin/rustc" ]] || die "rustc not found in Rust sysroot: ${RUST_SYSROOT_DIR}/bin/rustc"

mkdir -p "${OUTPUT_DIR}/input" "${OUTPUT_DIR}/logs"
INPUT_DIR="${OUTPUT_DIR}/input"
RESULTS_DIR="${OUTPUT_DIR}/results"
VENDOR_DIR="${OUTPUT_DIR}/third-party-vendor"

cp "${SDK_CRATE}" "${INPUT_DIR}/sdk.crate"
cp "${SERVER_ARCHIVE}" "${INPUT_DIR}/server.tar.gz"
cp "${SCRIPT_DIR}/candidate_registry.py" "${INPUT_DIR}/candidate_registry.py"
cp "${SCRIPT_DIR}/isolated_consumer.py" "${INPUT_DIR}/isolated_consumer.py"
cp "${SCRIPT_DIR}/consumer.rs" "${INPUT_DIR}/consumer.rs"

{
  echo "sdk_crate=${SDK_CRATE}"
  echo "server_archive=${SERVER_ARCHIVE}"
  echo "output_dir=${OUTPUT_DIR}"
  echo "image=${IMAGE}"
  echo "container=${CONTAINER_NAME}"
  echo "keep_running=${KEEP_RUNNING}"
  echo "keep_wait_seconds=${KEEP_WAIT_SECONDS}"
  echo "otlp=${OTLP_ENDPOINT}"
  echo "port_base=${PORT_BASE}"
  echo "rust_sysroot=${RUST_SYSROOT_DIR}"
  sha256sum "${INPUT_DIR}/sdk.crate" "${INPUT_DIR}/server.tar.gz"
} >"${OUTPUT_DIR}/acceptance-inputs.txt"

if [[ -n "${EXISTING_VENDOR_DIR}" ]]; then
  VENDOR_DIR="$(readlink -f "${EXISTING_VENDOR_DIR}")"
  [[ -d "${VENDOR_DIR}" ]] || die "vendor dir not found: ${VENDOR_DIR}"
else
  (
    cd "${SOURCE_DIR}"
    cargo vendor --locked --offline "${VENDOR_DIR}"
  ) >"${OUTPUT_DIR}/vendor-config.toml"
fi

DOCKER=(docker)
if ! "${DOCKER[@]}" ps >/dev/null 2>&1; then
  if command -v sudo >/dev/null 2>&1 && sudo -n docker ps >/dev/null 2>&1; then
    DOCKER=(sudo docker)
  else
    die "docker is not available for this user, and passwordless sudo docker is unavailable"
  fi
fi
printf '%q ' "${DOCKER[@]}" >"${OUTPUT_DIR}/docker-command.txt"
printf '\n' >>"${OUTPUT_DIR}/docker-command.txt"

if ! "${DOCKER[@]}" image inspect "${IMAGE}" >"${OUTPUT_DIR}/image-inspect-before.json" 2>/dev/null; then
  "${DOCKER[@]}" build \
    -t "${IMAGE}" \
    -f "${SCRIPT_DIR}/Dockerfile.acceptance" \
    "${SCRIPT_DIR}" \
    >"${OUTPUT_DIR}/image-build.log" 2>&1
fi
"${DOCKER[@]}" image inspect "${IMAGE}" >"${OUTPUT_DIR}/image-inspect.json"

if "${DOCKER[@]}" container inspect "${CONTAINER_NAME}" >/dev/null 2>&1; then
  die "container already exists: ${CONTAINER_NAME}"
fi

mkdir -p "${RESULTS_DIR}"
CONTAINER_CMD=(
  python3 /input/isolated_consumer.py
  --input /input
  --output /results
  --third-party-directory /third-party
  --otlp "${OTLP_ENDPOINT}"
  --port-base "${PORT_BASE}"
)
if [[ "${KEEP_RUNNING}" == "true" ]]; then
  CONTAINER_CMD=(bash -lc "$(printf '%q ' "${CONTAINER_CMD[@]}"); status=\$?; if [[ \${status} -eq 0 ]]; then sleep infinity; fi; exit \${status}")
fi

"${DOCKER[@]}" run -d \
  --name "${CONTAINER_NAME}" \
  --network host \
  --mount "type=bind,src=${INPUT_DIR},dst=/input,readonly" \
  --mount "type=bind,src=${RESULTS_DIR},dst=/results" \
  --mount "type=bind,src=${VENDOR_DIR},dst=/third-party,readonly" \
  --mount "type=bind,src=${RUST_SYSROOT_DIR},dst=/opt/rust,readonly" \
  "${IMAGE}" \
  "${CONTAINER_CMD[@]}" \
  >"${OUTPUT_DIR}/container-id.txt"

CONTAINER_ID="$(cat "${OUTPUT_DIR}/container-id.txt")"
"${DOCKER[@]}" container inspect "${CONTAINER_NAME}" >"${OUTPUT_DIR}/container-inspect-start.json"

if [[ "${KEEP_RUNNING}" == "true" ]]; then
  for _ in $(seq 1 "${KEEP_WAIT_SECONDS}"); do
    if [[ -f "${RESULTS_DIR}/consumer-result.json" ]]; then
      "${DOCKER[@]}" logs "${CONTAINER_NAME}" >"${OUTPUT_DIR}/container.log" 2>&1 || true
      "${DOCKER[@]}" container inspect "${CONTAINER_NAME}" >"${OUTPUT_DIR}/container-inspect-after-e2e.json" || true
      echo "基础隔离 E2E 已通过；container 保持运行供独立观测验收：${CONTAINER_NAME}"
      exit 0
    fi
    if [[ "$("${DOCKER[@]}" inspect -f '{{.State.Running}}' "${CONTAINER_NAME}")" != "true" ]]; then
      break
    fi
    sleep 1
  done
  "${DOCKER[@]}" logs "${CONTAINER_NAME}" >"${OUTPUT_DIR}/container.log" 2>&1 || true
  "${DOCKER[@]}" container inspect "${CONTAINER_NAME}" >"${OUTPUT_DIR}/container-inspect-failed.json" || true
  "${DOCKER[@]}" stop "${CONTAINER_NAME}" >/dev/null 2>&1 || true
  die "keep-running container did not reach base E2E success; see ${OUTPUT_DIR}/container.log"
fi

set +e
EXIT_CODE="$("${DOCKER[@]}" wait "${CONTAINER_NAME}")"
WAIT_STATUS="$?"
set -e
"${DOCKER[@]}" logs "${CONTAINER_NAME}" >"${OUTPUT_DIR}/container.log" 2>&1 || true
"${DOCKER[@]}" container inspect "${CONTAINER_NAME}" >"${OUTPUT_DIR}/container-inspect-final.json" || true
"${DOCKER[@]}" stop "${CONTAINER_NAME}" >/dev/null 2>&1 || true

if [[ "${WAIT_STATUS}" -ne 0 || "${EXIT_CODE}" -ne 0 ]]; then
  die "acceptance container failed: wait=${WAIT_STATUS} exit=${EXIT_CODE}; see ${OUTPUT_DIR}/container.log"
fi

echo "候选包隔离验收通过：${OUTPUT_DIR}"
