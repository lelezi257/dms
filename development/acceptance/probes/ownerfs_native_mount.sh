#!/bin/bash
# DEV mount/backend recovery evidence; not full OwnerFs/DFS acceptance or performance.
set -euo pipefail
if [ "$#" -ne 3 ]; then
  printf 'usage: %s TEST_BINARY FRESH_EXT4_EVIDENCE_DIRECTORY EXPECTED_BINARY_SHA256\n' "$0" >&2
  exit 64
fi
test "$(id -u)" -eq 0
binary="$(realpath -- "$1")"
evidence="$2"
expected="$3"
[[ "$evidence" = /* && "$evidence" != / && ! -e "$evidence" ]]
[[ "$expected" =~ ^[0-9a-f]{64}$ ]]
parent="$(realpath -- "$(dirname -- "$evidence")")"
test -d "$parent"
test "$(findmnt -n -o FSTYPE -T "$parent")" = ext4
test "$(sha256sum -- "$binary" | cut -d' ' -f1)" = "$expected"
umask 077
mkdir -- "$evidence"
mkdir -- "$evidence/tmp"
cat /proc/self/mountinfo > "$evidence/mountinfo.before"
{
  date -u --iso-8601=seconds
  uname -a
  python3 --version
  findmnt -n -o SOURCE,FSTYPE,OPTIONS,UUID -T "$parent"
  printf 'binary_sha256=%s\nparent_ns=%s\n' "$expected" "$(readlink /proc/self/ns/mnt)"
} > "$evidence/environment.txt"
export TMPDIR="$evidence/tmp" AFS_NATIVE_PRIVATE_NAMESPACE=1
set +e
timeout --signal=TERM --kill-after=5s 180s unshare --mount --propagation private --fork sh -c '
  printf "test_ns=%s\n" "$(readlink /proc/self/ns/mnt)"
  exec "$1" --include-ignored --nocapture
' native-tests "$binary" > "$evidence/tests.log" 2>&1
test_exit="$?"
set -e
printf '%s\n' "$test_exit" > "$evidence/tests.exit"
cat /proc/self/mountinfo > "$evidence/mountinfo.after"
cmp "$evidence/mountinfo.before" "$evidence/mountinfo.after"
test -z "$(find "$evidence/tmp" -mindepth 1 -maxdepth 1 -print -quit)"
cat "$evidence/environment.txt" "$evidence/tests.log"
exit "$test_exit"
