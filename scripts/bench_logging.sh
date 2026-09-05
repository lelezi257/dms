#!/usr/bin/env bash
set -euo pipefail

SCRIPTS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
source "${SCRIPTS_DIR}/_common.sh"

RECORDS="${1:-100000}"
MAX_FILE_SIZE="${2:-18446744073709551615}"
cargo run --quiet --release -p dms-logging --example logging_bench -- \
  "${RECORDS}" "${MAX_FILE_SIZE}"
