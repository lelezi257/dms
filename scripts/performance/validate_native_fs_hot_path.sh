#!/usr/bin/env bash
set -euo pipefail

contract="benchmarks/whitebox/native-fs-hot-path-contract.json"
results=()

while (($#)); do
  case "$1" in
    --contract) contract="$2"; shift 2 ;;
    --result) results+=("$2"); shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

python3 -m unittest scripts/performance/test_evaluate_native_fs_hot_path.py

if ((${#results[@]} == 0)); then
  results=(
    "evidence/native-fs-hot-path/run-1/result.json"
    "evidence/native-fs-hot-path/run-2/result.json"
  )
fi

python3 scripts/performance/evaluate_native_fs_hot_path.py \
  --contract "$contract" \
  "${results[@]}"
