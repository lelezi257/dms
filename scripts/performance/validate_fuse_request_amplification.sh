#!/usr/bin/env bash
set -euo pipefail

# 正式候选由三 VM harness 生成；本脚本只做确定性评价，不隐式启动或修改 VM。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RESULT="${DMS_FUSE_AMPLIFICATION_RESULT:-}"

python3 -m unittest \
  "$ROOT/scripts/performance/test_native_filesystem_workload.py" \
  "$ROOT/scripts/performance/test_assemble_native_filesystem_result.py" \
  "$ROOT/scripts/performance/test_evaluate_fuse_request_amplification.py"

if [[ -z "$RESULT" ]]; then
  echo "DMS_FUSE_AMPLIFICATION_RESULT is required" >&2
  exit 2
fi

python3 "$ROOT/scripts/performance/evaluate_native_filesystem.py" "$RESULT"
python3 "$ROOT/scripts/performance/evaluate_fuse_request_amplification.py" "$RESULT"
