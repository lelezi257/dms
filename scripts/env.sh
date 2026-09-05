#!/usr/bin/env bash

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "scripts/env.sh 只能在 dms-dev Linux 虚拟机内加载" >&2
  return 1 2>/dev/null || exit 1
fi

DMS_SOURCE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
export PATH="${HOME}/.cargo/bin:${PATH}"

echo "== 准备 Linux 开发环境 =="
if ! command -v cc >/dev/null 2>&1 \
  || ! command -v curl >/dev/null 2>&1 \
  || ! command -v protoc >/dev/null 2>&1; then
  echo "安装系统构建依赖（仅首次需要）……"
  sudo apt-get update
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
    build-essential \
    ca-certificates \
    curl \
    pkg-config \
    protobuf-compiler
fi

if ! command -v rustup >/dev/null 2>&1; then
  echo "安装 Rust 工具链管理器（仅首次需要）……"
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
    | sh -s -- -y --profile minimal --default-toolchain 1.95.0
fi

if ! rustc +1.95.0 --version >/dev/null 2>&1; then
  echo "安装 Rust 1.95.0（仅首次需要）……"
  rustup toolchain install 1.95.0 --profile minimal
fi

for component in rustfmt clippy; do
  if ! rustup component list --toolchain 1.95.0 --installed \
    | grep -q "^${component}-"; then
    rustup component add --toolchain 1.95.0 "${component}"
  fi
done

export DMS_SOURCE_DIR
export DMS_DEV_ENV=1
export RUST_BACKTRACE=1
export CARGO_TERM_COLOR=always
# Host RustRover 需要使用项目内默认 `source/target` 完成本机代码分析；
# Linux VM 必须使用 VM 私有目录，避免 macOS/Linux Cargo artifact 混在一起。
export DMS_CARGO_TARGET_DIR="${DMS_CARGO_TARGET_DIR:-${HOME}/.cache/dms/cargo-target}"
export CARGO_TARGET_DIR="${DMS_CARGO_TARGET_DIR}"
export DMS_RUNTIME_ROOT="${DMS_RUNTIME_ROOT:-/tmp/dms-local-dev}"
export DMS_NODE_ID="${DMS_NODE_ID:-dms-dev}"
export DMS_NODE_PORT="${DMS_NODE_PORT:-19000}"
export DMS_META_PORT="${DMS_META_PORT:-19100}"
export DMS_META_GRPC_PORT="${DMS_META_GRPC_PORT:-19300}"
export DMS_WORKER_PORT="${DMS_WORKER_PORT:-19200}"
export DMS_WORKER_UDS="${DMS_WORKER_UDS:-${DMS_RUNTIME_ROOT}/run/dms-worker.sock}"
export DMS_STAGING_TTL_MILLIS="${DMS_STAGING_TTL_MILLIS:-30000}"
export DMS_META_JOURNAL_DIR="${DMS_META_JOURNAL_DIR:-${DMS_RUNTIME_ROOT}/meta-journal}"
export DMS_LOG_LEVEL="${DMS_LOG_LEVEL:-info}"
export DMS_LOG_FORMAT="${DMS_LOG_FORMAT:-json}"
export DMS_LOG_MAX_FILE_SIZE_BYTES="${DMS_LOG_MAX_FILE_SIZE_BYTES:-268435456}"
export DMS_LOG_MAX_BACKUPS="${DMS_LOG_MAX_BACKUPS:-14}"
export DMS_LOG_MAX_AGE_SECONDS="${DMS_LOG_MAX_AGE_SECONDS:-604800}"
export DMS_TRACING_ENABLED="${DMS_TRACING_ENABLED:-false}"
export DMS_TRACING_PERIODIC_OPERATIONS="${DMS_TRACING_PERIODIC_OPERATIONS:-false}"
export DMS_TRACING_OTLP_ENDPOINT="${DMS_TRACING_OTLP_ENDPOINT:-http://127.0.0.1:4317}"
export DMS_TRACING_SAMPLE_RATIO="${DMS_TRACING_SAMPLE_RATIO:-0.01}"

cd "${DMS_SOURCE_DIR}" || return 1

echo "DMS Linux 开发环境已加载"
echo "  工作目录：${DMS_SOURCE_DIR}"
echo "  Rust：$(rustc --version)"
echo "  VM Cargo 产物：${CARGO_TARGET_DIR}"
echo "  下一步：./scripts/build.sh"
