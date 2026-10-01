#!/usr/bin/env bash
set -euo pipefail
test "$(uname -s)" = Linux
export PATH=/home/lzc.guest/.cargo/bin:$PATH
export CARGO_TARGET_DIR=/home/lzc.guest/afs-build/target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0
root=${1:?frozen root}
out=${2:?new output}
mode=${3:?local or full}
test ! -e "$out"
mkdir -p "$out"
cd "$root"
run() {
 local name=$1
 shift
 printf '%q ' "$@" > "$out/$name.command"
 printf '\n' >> "$out/$name.command"
 set +e
 "$@" > "$out/$name.log" 2>&1
 local result=$?
 set -e
 printf '%s\n' "$result" > "$out/$name.exit"
 tail -n 9 "$out/$name.log"
 return "$result"
}
case "$mode" in
local)
 run original env AFS_TEST_RDMA_DEVICE=rxe0 timeout 180 cargo test --offline --all-features --lib dfs_auto_prefers_rdma_replica_and_read_real_verbs -- --ignored --nocapture
 run peer timeout 180 cargo test --offline --all-features --lib node::rpc::peer:: -- --nocapture
 run control timeout 180 cargo test --offline --all-features --lib node::rpc::control::tests -- --nocapture
 run data timeout 180 cargo test --offline --all-features --lib node::rpc::data:: -- --nocapture
 ;;
full)
 bash "$root/development/evidence/20261001-owner-metrics/source-gate.sh" "$root" "$out/gate" full
 bash "$root/development/evidence/20261001-owner-metrics/source-gate.sh" "$root" "$out/features" features
 ;;
*) exit 2;;
esac
