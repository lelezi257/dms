#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
if [[ "${DMS_DEV_ENV:-}" == "1" && -f "${SCRIPTS_DIR}/_common.sh" ]]; then
  source "${SCRIPTS_DIR}/_common.sh"
else
  DMS_SOURCE_DIR="$(cd "${SCRIPTS_DIR}/.." && pwd -P)"
  DMS_LOG_DIR="${DMS_LOG_DIR:-${DMS_SOURCE_DIR}/log}"
fi

export DMS_LOG_DIR

# Bind mounts must exist before Docker Compose starts. Otherwise Docker creates
# the host directory as root and later DMS processes cannot open their logs.
mkdir -p "${DMS_LOG_DIR}"

ACTION="${1:-status}"
COMPOSE_FILE="${DMS_SOURCE_DIR}/infra/observability/compose.yaml"
COMPOSE_PROJECT="${DMS_OBSERVABILITY_PROJECT:-observability}"
COMPOSE_ARGS=(-p "${COMPOSE_PROJECT}" -f "${COMPOSE_FILE}")

if docker info >/dev/null 2>&1; then
  DOCKER=(docker)
elif sudo docker info >/dev/null 2>&1; then
  # Compose interpolation happens in the docker CLI process. Preserve only
  # the supported DMS overrides when the VM user needs sudo for /run/docker.sock.
  DOCKER=(
    sudo
    --preserve-env=DMS_PROMETHEUS_IMAGE,DMS_GRAFANA_IMAGE,DMS_LOKI_IMAGE,DMS_ALLOY_IMAGE,DMS_TEMPO_IMAGE,DMS_PROMETHEUS_PORT,DMS_GRAFANA_PORT,DMS_GRAFANA_USER,DMS_GRAFANA_PASSWORD,DMS_LOKI_PORT,DMS_ALLOY_PORT,DMS_OTLP_GRPC_PORT,DMS_OTLP_HTTP_PORT,DMS_TEMPO_PORT,DMS_LOKI_URL,DMS_ALLOY_INSTANCE,DMS_LOG_DIR
    docker
  )
else
  echo "Docker daemon unavailable. Install/start Docker inside this Linux VM first." >&2
  exit 2
fi

case "${ACTION}" in
  prepare-tempo-image)
    case "$(uname -m)" in
      aarch64 | arm64) tempo_arch="arm64" ;;
      x86_64 | amd64) tempo_arch="amd64" ;;
      *) echo "unsupported Tempo architecture: $(uname -m)" >&2; exit 2 ;;
    esac
    tempo_version="${DMS_TEMPO_VERSION:-2.8.2}"
    tempo_image="${DMS_TEMPO_LOCAL_IMAGE:-dms-tempo:${tempo_version}-local}"
    tempo_build_dir="$(mktemp -d)"
    trap 'rm -rf "${tempo_build_dir}"' EXIT
    curl --fail --location --show-error \
      "https://github.com/grafana/tempo/releases/download/v${tempo_version}/tempo_${tempo_version}_linux_${tempo_arch}.tar.gz" \
      --output "${tempo_build_dir}/tempo.tar.gz"
    tar -xzf "${tempo_build_dir}/tempo.tar.gz" -C "${tempo_build_dir}" tempo
    "${DOCKER[@]}" build \
      --file "${DMS_SOURCE_DIR}/infra/observability/tempo/Dockerfile" \
      --tag "${tempo_image}" \
      "${tempo_build_dir}"
    echo "Tempo fallback image prepared: ${tempo_image}"
    echo "Before 'up': export DMS_TEMPO_IMAGE=${tempo_image}"
    ;;
  up)
    "${DOCKER[@]}" compose "${COMPOSE_ARGS[@]}" up -d
    ;;
  agent-up)
    "${DOCKER[@]}" compose "${COMPOSE_ARGS[@]}" up -d alloy
    ;;
  down)
    "${DOCKER[@]}" compose "${COMPOSE_ARGS[@]}" down
    ;;
  status)
    "${DOCKER[@]}" compose "${COMPOSE_ARGS[@]}" ps
    ;;
  logs)
    "${DOCKER[@]}" compose "${COMPOSE_ARGS[@]}" logs --tail=100
    ;;
  *)
    echo "usage: ./scripts/observability.sh {prepare-tempo-image|up|agent-up|down|status|logs}" >&2
    exit 2
    ;;
esac
