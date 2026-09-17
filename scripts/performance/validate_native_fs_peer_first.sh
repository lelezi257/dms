#!/usr/bin/env bash
set -euo pipefail

contract="benchmarks/whitebox/native-fs-peer-first-contract.json"
run_one="evidence/native-fs-peer-first/candidate-run-1/result.json"
run_two="evidence/native-fs-peer-first/candidate-run-2/result.json"
output="evidence/native-fs-peer-first/evaluation.json"

while (($#)); do
  case "$1" in
    --contract) contract="$2"; shift 2 ;;
    --run-one) run_one="$2"; shift 2 ;;
    --run-two) run_two="$2"; shift 2 ;;
    --output) output="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

python3 scripts/performance/evaluate_native_fs_peer_first.py \
  --contract "$contract" \
  --output "$output" \
  "$run_one" "$run_two"
