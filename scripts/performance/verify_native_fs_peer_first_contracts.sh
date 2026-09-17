#!/usr/bin/env bash
set -euo pipefail

# P4 不接受只靠性能计数推断正确性。每个合同都由一个命名测试直接证明；
# 任何测试失败时脚本立即退出，因此不会生成带 true 的伪收据。
output=${1:-evidence/native-fs-peer-first/p4-contracts.json}

run_contract() {
  local test_name=$1
  # P4 的性能二进制和完整回归都使用 release profile。这里保持同一 profile，
  # 避免在容量有限的验证 VM 中再生成一套与最终证据无关的 debug 产物。
  cargo test --release -p dms-server --features fuse "$test_name" -- --nocapture
}

run_contract cold_current_read_pulls_all_missing_blocks_with_one_peer_stream
run_contract combined_plan_keeps_all_replica_facts_in_one_queue_item
run_contract import_rejects_corrupt_payload_when_peer_echoes_expected_checksum
run_contract resolved_version_without_live_replica_is_unavailable_not_not_found
run_contract import_switches_to_second_replica_when_first_source_is_unreachable
run_contract cached_stale_location_refreshes_exact_version_once
run_contract cached_location_read_retries_replica_report_outside_foreground_get

mkdir -p "$(dirname "$output")"
cat >"$output" <<'JSON'
{
  "schema": "dms.native-fs-peer-first-contract-receipt.v1",
  "foreground_synchronous_report_replicas": 0,
  "fault_contracts": {
    "checksum_mismatch_rejected": true,
    "missing_replica_rejected": true,
    "source_failure_falls_back": true,
    "stale_location_falls_back": true,
    "background_report_does_not_delay_read": true
  },
  "mechanism_contracts": {
    "multi_block_read_uses_one_peer_stream": true
  }
}
JSON

printf 'P4 contract receipt: %s\n' "$output"
