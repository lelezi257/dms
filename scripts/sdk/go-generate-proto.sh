#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != Linux ]]; then
  echo "Generate Go bindings inside the Linux development environment." >&2
  exit 1
fi

# 在 Linux 开发环境中从正式 proto 重新生成 Go SDK 私有 pb。
# 输出目录位于 sdk/go/internal/pb，用户包只导入 github.com/lelezi257/dms/sdk/go，
# 不需要安装 protoc，也不会在公开 API 中看到 protobuf DTO。

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
SOURCE_ROOT="$(cd -- "${SCRIPT_DIR}/../.." && pwd)"
PROTO_ROOT="${SOURCE_ROOT}/protocol/proto"
OUT_DIR="${SOURCE_ROOT}/sdk/go/internal/pb"
GO_PACKAGE="github.com/lelezi257/dms/sdk/go/internal/pb/dms/v1"

PROTO_FILES=(
  "dms/v1/types.proto"
  "dms/v1/client_node.proto"
)

GO_MAPPINGS=()
GO_GRPC_MAPPINGS=()
for proto_file in "${PROTO_FILES[@]}"; do
  GO_MAPPINGS+=("--go_opt=M${proto_file}=${GO_PACKAGE}")
  GO_GRPC_MAPPINGS+=("--go-grpc_opt=M${proto_file}=${GO_PACKAGE}")
done

mkdir -p "${OUT_DIR}"
# 先生成到独立临时目录；protoc/plugin 失败不能先删掉当前可用绑定。
GEN_TMP="$(mktemp -d "${OUT_DIR}.tmp.XXXXXX")"
trap 'rm -rf -- "$GEN_TMP"' EXIT

protoc \
  -I "${PROTO_ROOT}" \
  --go_out="${GEN_TMP}" \
  --go_opt=paths=source_relative \
  "${GO_MAPPINGS[@]}" \
  --go-grpc_out="${GEN_TMP}" \
  --go-grpc_opt=paths=source_relative \
  "${GO_GRPC_MAPPINGS[@]}" \
  "${PROTO_FILES[@]/#/${PROTO_ROOT}\/}"

mkdir -p "${OUT_DIR}/dms/v1"
cp "${GEN_TMP}/dms/v1/types.pb.go" "${GEN_TMP}/dms/v1/client_node.pb.go" \
   "${GEN_TMP}/dms/v1/client_node_grpc.pb.go" "${OUT_DIR}/dms/v1/"
# 仅清除曾误纳入 SDK 的两类生成文件，不删除目录中的其它源码。
rm -f -- "${OUT_DIR}/dms/v1/node_meta.pb.go" "${OUT_DIR}/dms/v1/node_meta_grpc.pb.go" \
         "${OUT_DIR}/dms/v1/node_peer.pb.go" "${OUT_DIR}/dms/v1/node_peer_grpc.pb.go"
