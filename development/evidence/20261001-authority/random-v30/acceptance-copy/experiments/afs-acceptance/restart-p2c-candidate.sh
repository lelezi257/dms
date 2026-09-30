#!/usr/bin/env bash
set -euo pipefail
service=${1:?meta or node}
expected=${2:?expected current executable sha256}
case "$service" in
  meta) run=/mnt/lima-afsctlstate/afs-delivery/p1b; binary=afs-meta; health=http://127.0.0.1:17501/health ;;
  node) run=/mnt/lima-afsadata/afs-delivery/p1b; binary=afs-node; health=http://127.0.0.1:17401/health; [ "$(id -u)" = 0 ] || { echo 'node restart requires root for caller ownership'; exit 1; } ;;
  *) exit 2 ;;
esac
pid=$(cat "$run/run/$service.pid")
exe=$(readlink "/proc/$pid/exe")
[ "$exe" = "$run/bin/$binary" ] || { echo "unexpected current executable $exe"; exit 1; }
actual=$(sha256sum "/proc/$pid/exe" | cut -d ' ' -f1)
[ "$actual" = "$expected" ] || { echo "current binary identity mismatch $actual"; exit 1; }
stamp=$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p "$run/previous/$stamp"
cp "$run/bin/$binary" "$run/logs/$service.log" "$run/previous/$stamp/"
printf 'old_pid=%s old_sha=%s\n' "$pid" "$actual"
kill -INT "$pid"
for attempt in {1..150}; do
  [ ! -d "/proc/$pid" ] && break
  state=$(awk '/^State:/ {print $2}' "/proc/$pid/status")
  [ "$state" = Z ] && break
  sleep .2
done
if [ -d "/proc/$pid" ] && [ "$(awk '/^State:/ {print $2}' "/proc/$pid/status")" != Z ]; then
  echo 'graceful shutdown did not finish'; exit 1
fi
if [ "$service" = node ]; then
  for mount in "$run/mount-ownerfs" "$run/mount-dfs"; do
    ! mountpoint -q "$mount" || { echo "mount still active: $mount"; exit 1; }
  done
fi
mv "$run/bin/$binary.p2c" "$run/bin/$binary"
chmod 755 "$run/bin/$binary"
nohup "$run/bin/$binary" --config "$run/$service.toml" > "$run/logs/$service.log" 2>&1 &
new_pid=$!
echo "$new_pid" > "$run/run/$service.pid"
for attempt in {1..100}; do
  kill -0 "$new_pid"
  ready=false
  if curl -fsS "$health" > "$run/logs/$service-health.json"; then
    if [ "$service" = meta ] || { mountpoint -q "$run/mount-ownerfs" && mountpoint -q "$run/mount-dfs"; }; then ready=true; fi
  fi
  if [ "$ready" = true ]; then
    printf 'new_pid=%s\n' "$new_pid"
    sha256sum "/proc/$new_pid/exe"
    cat "$run/logs/$service-health.json"
    [ "$service" = meta ] || findmnt -rn -t fuse -o SOURCE,FSTYPE,TARGET,OPTIONS
    exit 0
  fi
  sleep .2
done
exit 1
