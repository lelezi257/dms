#!/usr/bin/env bash
set -euo pipefail

contract="benchmarks/whitebox/native-fs-write-through-contract.json"
output="evidence/native-fs-write-through/evaluation.json"
results=(
  "evidence/native-fs-write-through/run-1/result.json"
  "evidence/native-fs-write-through/run-2/result.json"
)

python3 scripts/performance/evaluate_native_fs_write_through.py \
  --contract "$contract" \
  --output "$output" \
  "${results[@]}"
