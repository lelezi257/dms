#!/usr/bin/env bash
set -euo pipefail

# 机器门禁入口。正式候选由三 VM harness 写入 DMS_NATIVE_FS_RESULT；
# 这里只负责确定性评价和 evaluator 自测，不隐式启动 VM 或修改环境。
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RESULT="${DMS_NATIVE_FS_RESULT:-}"

python3 -m unittest "$ROOT/scripts/performance/test_evaluate_native_filesystem.py"

if [[ -z "$RESULT" ]]; then
  echo "DMS_NATIVE_FS_RESULT is required" >&2
  exit 2
fi

python3 "$ROOT/scripts/performance/evaluate_native_filesystem.py" "$RESULT"
