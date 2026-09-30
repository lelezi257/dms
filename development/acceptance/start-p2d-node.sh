#!/usr/bin/env bash
set -euo pipefail
node=${1:?a or b}
case "$node" in a) ip=192.168.109.12; volume=/mnt/lima-afsadata;; b) ip=192.168.109.13; volume=/mnt/lima-afsbdata;; *) exit 2;; esac
run=$volume/afs-delivery/p1b
[ "$(id -u)" = 0 ] || { echo 'Node requires root to enforce original caller ownership'; exit 1; }
[ "$(findmnt -T "$run" -n -o FSTYPE)" = ext4 ] || exit 1
if [ -f "$run/run/node.pid" ] && kill -0 "$(cat "$run/run/node.pid")" 2>/dev/null; then echo 'existing Node must be stopped explicitly'; exit 1; fi
for peer in a b; do [ -s "$run/tls/node-$peer.pem" ] || exit 1; done
cat > "$run/node.toml" <<CONFIG
id = "node-$node"
fs = "all"
meta_endpoint = "https://192.168.109.11:17500"
advertise_endpoint = "https://$ip:17400"
grpc_listen = "0.0.0.0:17400"
rest_listen = "0.0.0.0:17401"
data_dir = "$run/state/node"
uds_path = "$run/run/node.sock"
ownerfs_mount = "$run/mount-ownerfs"
dfs_mount = "$run/mount-dfs"
data_mode = "grpc"
tls_ca_certificate = "$run/tls/ca.pem"
tls_identity_certificate = "$run/tls/node-$node.pem"
tls_identity_private_key = "$run/tls/node-$node-key.pem"
tls_server_name = "afs-cluster"
trusted_node_certs = { node-a = "$run/tls/node-a.pem", node-b = "$run/tls/node-b.pem" }
log_level = "info"
trace_enabled = false
CONFIG
if [ "${2:-}" = --prepare-config ]; then exit 0; fi
nohup "$run/bin/afs-node" --config "$run/node.toml" > "$run/logs/node.log" 2>&1 &
pid=$!
echo "$pid" > "$run/run/node.pid"
for attempt in {1..100}; do
  kill -0 "$pid"
  if curl -fsS http://127.0.0.1:17401/health > "$run/logs/node-health.json" && mountpoint -q "$run/mount-dfs" && mountpoint -q "$run/mount-ownerfs"; then
    sha256sum "/proc/$pid/exe"
    cat "$run/logs/node-health.json"
    findmnt -rn -t fuse -o SOURCE,FSTYPE,TARGET,OPTIONS
    exit 0
  fi
  sleep .2
done
exit 1
