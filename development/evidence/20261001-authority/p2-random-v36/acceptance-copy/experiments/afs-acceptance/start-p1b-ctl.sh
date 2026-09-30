#!/usr/bin/env bash
set -euo pipefail
# Run on dedicated ctl only; experimental services own these explicit paths.
RUN=/mnt/lima-afsctlstate/afs-delivery/p1b
sudo install -d -o "$(id -u)" -g "$(id -g)" "$RUN"
mkdir -p "$RUN"/{bin,tls,state,run,logs}
for service in etcd meta; do
  if [ -f "$RUN/run/$service.pid" ] && kill -0 "$(cat "$RUN/run/$service.pid")" 2>/dev/null; then
    echo "existing $service must be stopped explicitly before starting" >&2; exit 1
  fi
done
if [ ! -f "$RUN/tls/ca.pem" ]; then
  umask 077
  openssl req -x509 -newkey rsa:2048 -nodes -days 30 -subj /CN=afs-acceptance-ca -keyout "$RUN/tls/ca-key.pem" -out "$RUN/tls/ca.pem" 2>/dev/null
  openssl req -new -newkey rsa:2048 -nodes -subj /CN=afs-meta -keyout "$RUN/tls/meta-key.pem" -out "$RUN/tls/meta.csr" 2>/dev/null
  printf '%s\n' 'subjectAltName=DNS:afs-cluster,DNS:afs-meta,IP:192.168.109.11' 'extendedKeyUsage=serverAuth,clientAuth' > "$RUN/tls/meta.ext"
  openssl x509 -req -in "$RUN/tls/meta.csr" -CA "$RUN/tls/ca.pem" -CAkey "$RUN/tls/ca-key.pem" -CAcreateserial -days 30 -extfile "$RUN/tls/meta.ext" -out "$RUN/tls/meta.pem" 2>/dev/null
fi
if [ "${1:-}" = --prepare-tls ]; then exit 0; fi
[ -f "$RUN/tls/node-a.pem" ] || { echo 'sign Node A CSR before starting ctl'; exit 1; }
cat > "$RUN/meta.toml" <<EOF
id = "accept-meta-ctl"
fs = "all"
meta_store = "etcd"
etcd_endpoint = "http://127.0.0.1:2379"
data_dir = "$RUN/state/meta"
uds_path = "$RUN/run/meta.sock"
grpc_listen = "0.0.0.0:17500"
rest_listen = "0.0.0.0:17501"
tls_ca_certificate = "$RUN/tls/ca.pem"
tls_identity_certificate = "$RUN/tls/meta.pem"
tls_identity_private_key = "$RUN/tls/meta-key.pem"
tls_server_name = "afs-cluster"
trusted_node_certs = { node-a = "$RUN/tls/node-a.pem" }
log_level = "info"
trace_enabled = false
EOF
ETCD_UNSUPPORTED_ARCH=arm64 nohup etcd --name afs-p1b-etcd --data-dir "$RUN/state/etcd" --listen-client-urls http://127.0.0.1:2379 --advertise-client-urls http://127.0.0.1:2379 --listen-peer-urls http://127.0.0.1:2380 --initial-advertise-peer-urls http://127.0.0.1:2380 --initial-cluster afs-p1b-etcd=http://127.0.0.1:2380 --max-request-bytes 67108864 > "$RUN/logs/etcd.log" 2>&1 &
echo $! > "$RUN/run/etcd.pid"
for attempt in {1..50}; do
  if ETCDCTL_API=3 etcdctl --endpoints http://127.0.0.1:2379 endpoint health > "$RUN/logs/etcd-health.log" 2>&1; then break; fi
  sleep .2
done
ETCDCTL_API=3 etcdctl --endpoints http://127.0.0.1:2379 endpoint health
nohup "$RUN/bin/afs-meta" --config "$RUN/meta.toml" > "$RUN/logs/meta.log" 2>&1 &
echo $! > "$RUN/run/meta.pid"
for attempt in {1..50}; do
  if curl -fsS http://127.0.0.1:17501/health > "$RUN/logs/meta-health.json"; then cat "$RUN/logs/meta-health.json"; exit 0; fi
  kill -0 "$(cat "$RUN/run/meta.pid")"
  sleep .2
done
exit 1
