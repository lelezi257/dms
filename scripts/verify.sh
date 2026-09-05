#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

echo "== 1/8 format =="
cargo fmt --all -- --check

echo "== 2/8 clippy =="
cargo clippy --workspace --all-targets --locked -- -D warnings

echo "== 3/8 unit tests =="
cargo test --workspace --locked

echo "== 4/8 packaged SDK consumer =="
# 从真实 SDK 包编译统一消费者；比源码 path 检查更完整，不需要独立测试工程。
# 这里只构包并编译，候选仓下载及 TCP/SHM 运行由 scripts/release/accept.sh 验证。
python3 "${SCRIPTS_DIR}/package_sdk.py"

echo "== 5/8 deployed process checks =="
"${SCRIPTS_DIR}/status.sh"

echo "== 6/8 Client -> Node UDS/TCP E2E =="
DMS_ENDPOINT="unix://${DMS_WORKER_UDS}" "${CARGO_TARGET_DIR}/release/examples/sdk_api"
DMS_ENDPOINT="http://127.0.0.1:${DMS_WORKER_PORT}" "${CARGO_TARGET_DIR}/release/examples/sdk_api"

echo "== 7/8 Node -> Node / Node -> Meta TCP gRPC E2E =="
cargo test -p dms-server --locked \
  node::peer_service::tests::node_to_node_probe_crosses_grpc_and_shared_node_owner -- --exact
cargo test -p dms-server --locked \
  meta::metadata_service::tests::node_to_meta_session_crosses_grpc_mailbox_and_oneshot -- --exact

echo "== 8/8 startup contracts and observability =="
"${SCRIPTS_DIR}/verify_metrics.sh"
if "${DMS_BIN_DIR}/dms-meta" serve --node-id invalid-meta \
  >"${DMS_LOG_DIR}/expected-meta-startup-failure.log" 2>&1; then
  echo "dms-meta 缺少业务 listener 时意外启动成功" >&2
  exit 1
fi
grep -q -- "requires --grpc-address" \
  "${DMS_LOG_DIR}/expected-meta-startup-failure.log" \
  || grep -q -- "missing required config field \`grpc_address\`" \
    "${DMS_LOG_DIR}/expected-meta-startup-failure.log"

if "${DMS_BIN_DIR}/dms-node" serve --node-id invalid-node \
  >"${DMS_LOG_DIR}/expected-node-startup-failure.log" 2>&1; then
  echo "dms-node 缺少业务 listener/Meta 时意外启动成功" >&2
  exit 1
fi
grep -q -- "requires --worker-tcp-address or --worker-uds-path" \
  "${DMS_LOG_DIR}/expected-node-startup-failure.log" \
  || grep -q -- "missing required config field \`worker_tcp_address or worker_uds_path\`" \
    "${DMS_LOG_DIR}/expected-node-startup-failure.log"

# Arena lifecycle 已由 typed Prometheus Metrics 验证；结构化日志不再重复
# 模拟指标，避免日志和指标成为两套相互漂移的事实来源。

if "${DMS_BIN_DIR}/dms-health" health \
  --address "127.0.0.1:${DMS_NODE_PORT}" \
  --expect-component dms-meta \
  --expect-node "${DMS_NODE_ID}"; then
  echo "反向身份检查意外成功" >&2
  exit 1
else
  echo "反向身份检查按预期失败"
fi

echo "全部验证通过"
