#!/usr/bin/env bash

if [[ "${DMS_DEV_ENV:-}" != "1" ]]; then
  echo "请先在 dms-dev 中执行：source scripts/env.sh" >&2
  exit 2
fi

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "该命令只能在 Linux 虚拟机中运行" >&2
  exit 2
fi

cd "${DMS_SOURCE_DIR}"

if [[ -z "${CARGO_TARGET_DIR:-}" ]]; then
  echo "CARGO_TARGET_DIR 未设置；请重新执行：source scripts/env.sh" >&2
  exit 2
fi

DMS_BIN_DIR="${DMS_RUNTIME_ROOT}/bin"
DMS_LOG_DIR="${DMS_RUNTIME_ROOT}/log"
DMS_RUN_DIR="${DMS_RUNTIME_ROOT}/run"

export DMS_BIN_DIR DMS_LOG_DIR DMS_RUN_DIR
export CARGO_TARGET_DIR
export DMS_META_GRPC_PORT DMS_WORKER_PORT DMS_WORKER_UDS DMS_META_JOURNAL_DIR
export DMS_LOG_LEVEL DMS_LOG_FORMAT DMS_LOG_MAX_FILE_SIZE_BYTES
export DMS_LOG_MAX_BACKUPS DMS_LOG_MAX_AGE_SECONDS
export DMS_TRACING_ENABLED DMS_TRACING_PERIODIC_OPERATIONS DMS_TRACING_OTLP_ENDPOINT DMS_TRACING_SAMPLE_RATIO

stop_process() {
  local process="$1"
  local pid_file="${DMS_RUN_DIR}/${process}.pid"
  local expected_exe="${DMS_BIN_DIR}/${process}"

  [[ -r "${pid_file}" ]] || return 0

  local pid actual_exe
  pid="$(cat "${pid_file}")"
  case "${pid}" in
    ''|*[!0-9]*)
      echo "${process}: 非法 pid 文件 ${pid_file}" >&2
      return 1
      ;;
  esac

  actual_exe="$(readlink -f "/proc/${pid}/exe" 2>/dev/null || true)"
  actual_exe="${actual_exe% (deleted)}"
  if [[ "${actual_exe}" == "${expected_exe}" ]]; then
    kill "${pid}"
    for _ in 1 2 3 4 5 6 7 8 9 10; do
      kill -0 "${pid}" 2>/dev/null || break
      sleep 0.1
    done
  fi
  rm -f "${pid_file}"
}
