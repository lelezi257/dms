#!/usr/bin/env bash
set -euo pipefail
# Dedicated acceptance ctl only; retain the existing backend and record identity.
[ "$(id -u)" = 0 ] || exit 1
run=/mnt/lima-afsctlstate/afs-delivery/p1b
expected_pid=${1:?expected current etcd PID}
pid=$(cat "$run/run/etcd.pid")
[ "$pid" = "$expected_pid" ] || { echo 'etcd PID changed'; exit 1; }
[ "$(readlink /proc/"$pid"/exe)" = /usr/bin/etcd ] || exit 1
tr '\0' ' ' < /proc/"$pid"/cmdline | tee "$run/logs/etcd-before-limit.cmdline"
echo
grep -q -- "--data-dir $run/state/etcd" "$run/logs/etcd-before-limit.cmdline"
stamp=$(date -u +%Y%m%dT%H%M%SZ)
backup=$run/previous/etcd-limit-$stamp
mkdir -p "$backup"
cp "$run/logs/etcd.log" "$backup/"
ETCDCTL_API=3 etcdctl --endpoints=http://127.0.0.1:2379 snapshot save "$backup/etcd.snapshot"
ETCDCTL_API=3 etcdctl --endpoints=http://127.0.0.1:2379 get /afs/meta/snapshot --print-value-only | sha256sum > "$backup/snapshot-value-before.sha256"
kill -TERM "$pid"
for attempt in {1..100}; do
  if [ ! -d /proc/"$pid" ] || [ "$(awk '/^State:/ {print $2}' /proc/"$pid"/status)" = Z ]; then break; fi
  sleep .2
done
if [ -d /proc/"$pid" ] && [ "$(awk '/^State:/ {print $2}' /proc/"$pid"/status)" != Z ]; then
  echo 'etcd did not stop gracefully'; exit 1
fi
ETCD_UNSUPPORTED_ARCH=arm64 nohup /usr/bin/etcd --name afs-p1b-etcd --data-dir "$run/state/etcd" --listen-client-urls http://127.0.0.1:2379 --advertise-client-urls http://127.0.0.1:2379 --listen-peer-urls http://127.0.0.1:2380 --initial-advertise-peer-urls http://127.0.0.1:2380 --initial-cluster afs-p1b-etcd=http://127.0.0.1:2380 --max-request-bytes 67108864 > "$run/logs/etcd.log" 2>&1 &
new_pid=$!
echo "$new_pid" > "$run/run/etcd.pid"
for attempt in {1..100}; do
  kill -0 "$new_pid"
  if ETCDCTL_API=3 etcdctl --endpoints=http://127.0.0.1:2379 endpoint health > "$backup/health-after.log" 2>&1; then
    ETCDCTL_API=3 etcdctl --endpoints=http://127.0.0.1:2379 get /afs/meta/snapshot --print-value-only | sha256sum > "$backup/snapshot-value-after.sha256"
    cmp "$backup/snapshot-value-before.sha256" "$backup/snapshot-value-after.sha256"
    printf 'new_pid=%s max_request_bytes=67108864 data_dir=%s backup=%s\n' "$new_pid" "$run/state/etcd" "$backup"
    sha256sum /proc/"$new_pid"/exe
    tr '\0' ' ' < /proc/"$new_pid"/cmdline
    echo
    cat "$backup/health-after.log" "$backup/snapshot-value-after.sha256"
    exit 0
  fi
  sleep .2
done
exit 1
