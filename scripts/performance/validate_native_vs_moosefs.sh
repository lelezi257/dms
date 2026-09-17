#!/usr/bin/env bash
set -euo pipefail

contract="benchmarks/whitebox/native-vs-moosefs-contract.json"
result="evidence/native-vs-moosefs/latest/result.json"

while (($#)); do
  case "$1" in
    --contract) contract="$2"; shift 2 ;;
    --result) result="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

python3 scripts/performance/evaluate_native_vs_moosefs.py \
  --contract "$contract" \
  "$result"
