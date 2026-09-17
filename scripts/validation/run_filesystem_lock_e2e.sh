#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT_DIR="${DMS_FILESYSTEM_LOCK_E2E_OUT_DIR:-"$ROOT/../evidence/filesystem-lock/$(date -u +%Y%m%dT%H%M%SZ)"}"

DMS_FILESYSTEM_E2E_OUT_DIR="$OUT_DIR" \
  bash "$ROOT/scripts/validation/run_filesystem_shared_file_e2e.sh"

python3 - "$OUT_DIR/distributed-locks.json" <<'PY'
import json
import sys
from pathlib import Path

value = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
expected = {
    "cross_node_getlk",
    "split_unlock",
    "shared_compatibility",
    "cross_node_flock",
    "blocking_wakeup",
    "interrupt_cancels_waiter",
}
actual = set(value.get("checks", []))
if value.get("status") != "passed" or actual != expected:
    raise SystemExit(f"distributed lock evidence mismatch: {value!r}")
PY

echo "$OUT_DIR"
