#!/usr/bin/env bash
set -euo pipefail
RUN=/mnt/lima-afsadata/afs-delivery/p1b
sudo install -d -o "$(id -u)" -g "$(id -g)" "$RUN"
mkdir -p "$RUN"/{bin,tls,state,run,logs,mount-dfs,mount-ownerfs}
if [ -f "$RUN/run/node.pid" ] && kill -0 "$(cat "$RUN/run/node.pid")" 2>/dev/null; then
  echo 'existing Node must be stopped explicitly before starting' >&2; exit 1
fi
cat > "$RUN/node.toml" <<EOF
id = "node-a"
fs = "all"
meta_endpoint = "https://192.168.109.11:17500"
advertise_endpoint = "https://192.168.109.12:17400"
grpc_listen = "0.0.0.0:17400"
rest_listen = "0.0.0.0:17401"
data_dir = "$RUN/state/node"
uds_path = "$RUN/run/node.sock"
ownerfs_mount = "$RUN/mount-ownerfs"
dfs_mount = "$RUN/mount-dfs"
data_mode = "grpc"
tls_ca_certificate = "$RUN/tls/ca.pem"
tls_identity_certificate = "$RUN/tls/node-a.pem"
tls_identity_private_key = "$RUN/tls/node-a-key.pem"
tls_server_name = "afs-cluster"
trusted_node_certs = { node-a = "$RUN/tls/node-a.pem" }
log_level = "info"
trace_enabled = false
EOF
nohup "$RUN/bin/afs-node" --config "$RUN/node.toml" > "$RUN/logs/node.log" 2>&1 &
echo $! > "$RUN/run/node.pid"
for attempt in {1..100}; do
  kill -0 "$(cat "$RUN/run/node.pid")"
  if curl -fsS http://127.0.0.1:17401/health > "$RUN/logs/node-health.json" && mountpoint -q "$RUN/mount-dfs" && mountpoint -q "$RUN/mount-ownerfs"; then
    findmnt -rn --mountpoint "$RUN/mount-dfs" -o SOURCE,FSTYPE,TARGET
    findmnt -rn --mountpoint "$RUN/mount-ownerfs" -o SOURCE,FSTYPE,TARGET
    cat "$RUN/logs/node-health.json"
    exit 0
  fi
  sleep .2
done
exit 1
