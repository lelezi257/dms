#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
PROJECT_ROOT="$(cd "${SCRIPTS_DIR}/../.." && pwd -P)"
VM_CONFIG="${SCRIPTS_DIR}/lima.yaml"
VM_NAME="${DMS_DEV_VM:-dms-dev}"
DMS_LIMA_HOME="${DMS_LIMA_HOME:-${HOME}/.lima}"

lima() {
  env LIMA_HOME="${DMS_LIMA_HOME}" limactl "$@"
}

instance_exists() {
  [[ -d "${DMS_LIMA_HOME}/${VM_NAME}" ]]
}

instance_status() {
  lima list "${VM_NAME}" --format '{{.Status}}' 2>/dev/null || true
}

usage() {
  cat <<EOF
用法：
  ./source/scripts/vm.sh up       创建并启动开发虚拟机
  ./source/scripts/vm.sh shell    进入虚拟机中的源码目录
  ./source/scripts/vm.sh status   查看虚拟机状态
  ./source/scripts/vm.sh stop     停止虚拟机
EOF
}

case "${1:-}" in
  up)
    mkdir -p "${DMS_LIMA_HOME}"
    if ! instance_exists; then
      echo "首次创建 ${VM_NAME}……"
      lima create --tty=false \
        --name "${VM_NAME}" \
        --set ".mounts[0].location = \"${PROJECT_ROOT}\"" \
        "${VM_CONFIG}"
    fi

    if [[ "$(instance_status)" != "Running" ]]; then
      echo "启动 ${VM_NAME}……"
      lima start --tty=false "${VM_NAME}"
    fi

    if ! lima shell --workdir /workspace/dms/source "${VM_NAME}" -- \
      test -f Cargo.toml; then
      echo "${VM_NAME}: /workspace/dms 没有正确挂载当前项目" >&2
      exit 1
    fi

    echo "${VM_NAME} 已就绪。下一步：./source/scripts/vm.sh shell"
    ;;

  shell)
    if [[ "$(instance_status)" != "Running" ]]; then
      echo "${VM_NAME} 尚未启动，请先执行：./source/scripts/vm.sh up" >&2
      exit 2
    fi
    exec env LIMA_HOME="${DMS_LIMA_HOME}" \
      limactl shell --workdir /workspace/dms/source "${VM_NAME}"
    ;;

  status)
    if instance_exists; then
      lima list "${VM_NAME}"
    else
      echo "${VM_NAME}: Not created"
    fi
    ;;

  stop)
    if [[ "$(instance_status)" == "Running" ]]; then
      lima stop "${VM_NAME}"
    else
      echo "${VM_NAME}: already stopped"
    fi
    ;;

  *)
    usage
    exit 2
    ;;
esac
