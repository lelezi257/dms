#!/usr/bin/env bash
set -euo pipefail

contract="benchmarks/whitebox/native-fs-write-through-contract.json"
output="evidence/native-fs-write-through/evaluation.json"
results=(
  "${1:-evidence/native-fs-write-through/candidate-run-1/result.json}"
  "${2:-evidence/native-fs-write-through/candidate-run-2/result.json}"
)

python3 scripts/performance/evaluate_native_fs_write_through.py \
  --contract "$contract" \
  --output "$output" \
  "${results[@]}"
