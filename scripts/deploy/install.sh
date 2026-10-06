#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: install.sh [options]

Options:
  --prefix DIR        Program install prefix. Default: /opt/afs
  --config-dir DIR    Config directory. Default: /etc/afs
  --state-dir DIR     Persistent state directory. Default: /var/lib/afs
  --run-dir DIR       Runtime PID directory. Default: /run/afs
  --log-dir DIR       Log directory. Default: /var/log/afs
  --mount-root DIR    Mount root for generated configs. Default: /mnt/afs
  --start SERVICE     Start service after install: meta, node, dfs, ownerfs, all.
  --no-verify         Skip package SHA256SUMS verification.
  -h, --help          Show this help.

The installer preserves existing config and data by default.
USAGE
}

PREFIX=/opt/afs
CONFIG_DIR=/etc/afs
STATE_DIR=/var/lib/afs
RUN_DIR=/run/afs
LOG_DIR=/var/log/afs
MOUNT_ROOT=/mnt/afs
START_SERVICE=
VERIFY=1

while [ "$#" -gt 0 ]; do
  case "$1" in
    --prefix) PREFIX=${2:-}; shift 2 ;;
    --config-dir) CONFIG_DIR=${2:-}; shift 2 ;;
    --state-dir) STATE_DIR=${2:-}; shift 2 ;;
    --run-dir) RUN_DIR=${2:-}; shift 2 ;;
    --log-dir) LOG_DIR=${2:-}; shift 2 ;;
    --mount-root) MOUNT_ROOT=${2:-}; shift 2 ;;
    --start) START_SERVICE=${2:-}; shift 2 ;;
    --no-verify) VERIFY=0; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ "$(id -u)" -ne 0 ]; then
  echo "install.sh must run as root" >&2
  exit 1
fi

case "$START_SERVICE" in
  ""|meta|node|dfs|ownerfs|all) ;;
  *) echo "--start must be meta, node, dfs, ownerfs or all" >&2; exit 2 ;;
esac

require_absolute_dir_arg() {
  name=$1
  value=$2
  case "$value" in
    /*) ;;
    *) echo "$name must be an absolute path: $value" >&2; exit 2 ;;
  esac
  if [ "$value" = / ]; then
    echo "$name must not be /" >&2
    exit 2
  fi
}

require_absolute_dir_arg --prefix "$PREFIX"
require_absolute_dir_arg --config-dir "$CONFIG_DIR"
require_absolute_dir_arg --state-dir "$STATE_DIR"
require_absolute_dir_arg --run-dir "$RUN_DIR"
require_absolute_dir_arg --log-dir "$LOG_DIR"
require_absolute_dir_arg --mount-root "$MOUNT_ROOT"

PACKAGE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "$VERIFY" -eq 1 ]; then
  if [ ! -f "$PACKAGE_DIR/SHA256SUMS" ]; then
    echo "missing package SHA256SUMS" >&2
    exit 1
  fi
  (cd "$PACKAGE_DIR" && sha256sum -c SHA256SUMS)
fi

for path in "$PREFIX/bin" "$CONFIG_DIR" "$STATE_DIR" "$RUN_DIR" "$LOG_DIR" "$MOUNT_ROOT"; do
  mkdir -p "$path"
done

install -m 0755 "$PACKAGE_DIR/bin/afs-meta" "$PREFIX/bin/afs-meta"
install -m 0755 "$PACKAGE_DIR/bin/afs-node" "$PREFIX/bin/afs-node"
install -m 0755 "$PACKAGE_DIR/bin/afs-processctl" "$PREFIX/bin/afs-processctl"
install -m 0755 "$PACKAGE_DIR/bin/afs-trial-config" "$PREFIX/bin/afs-trial-config"
install -m 0755 "$PACKAGE_DIR/bin/afs-selfcheck" "$PREFIX/bin/afs-selfcheck"
install -m 0755 "$PACKAGE_DIR/bin/dep02-smoke.sh" "$PREFIX/bin/dep02-smoke.sh"
install -m 0644 "$PACKAGE_DIR/manifest.json" "$PREFIX/manifest.json"
install -m 0644 "$PACKAGE_DIR/DEPENDENCIES.md" "$PREFIX/DEPENDENCIES.md"
mkdir -p "$PREFIX/docs/guides"
install -m 0644 "$PACKAGE_DIR/docs/guides/trial.md" "$PREFIX/docs/guides/trial.md"

write_config_once() {
  src=$1
  dst=$2
  if [ -e "$dst" ]; then
    echo "preserve existing config: $dst"
    return
  fi
  sed \
    -e "s#/var/lib/afs#$STATE_DIR#g" \
    -e "s#/mnt/afs#$MOUNT_ROOT#g" \
    -e "s#/etc/afs#$CONFIG_DIR#g" \
    -e "s#/run/afs#$RUN_DIR#g" \
    "$src" >"$dst"
  chmod 0644 "$dst"
  echo "created config: $dst"
}

bootstrap_tls_once() {
  tls_dir="$CONFIG_DIR/tls"
  mkdir -p "$tls_dir"
  if [ -f "$tls_dir/ca.pem" ] && [ -f "$tls_dir/meta.pem" ] && [ -f "$tls_dir/node-a.pem" ]; then
    echo "preserve existing TLS bootstrap files: $tls_dir"
    return
  fi
  command -v openssl >/dev/null 2>&1 || {
    echo "openssl is required to bootstrap local TLS; install openssl or provide $tls_dir certificates" >&2
    exit 1
  }
  umask 077
  openssl genrsa -out "$tls_dir/ca-key.pem" 2048 >/dev/null 2>&1
  openssl req -x509 -new -nodes -key "$tls_dir/ca-key.pem" -sha256 -days 3650 -subj "/CN=afs-local-ca" -out "$tls_dir/ca.pem" >/dev/null 2>&1
  for name in meta node-a; do
    cn=afs-$name
    [ "$name" = meta ] && cn=afs-meta
    openssl genrsa -out "$tls_dir/$name-key.pem" 2048 >/dev/null 2>&1
    openssl req -new -key "$tls_dir/$name-key.pem" -subj "/CN=$cn" -out "$tls_dir/$name.csr" >/dev/null 2>&1
    cat > "$tls_dir/$name.ext" <<EOF_TLS
subjectAltName = DNS:$cn,DNS:afs-meta,DNS:node-a,IP:127.0.0.1
extendedKeyUsage = serverAuth,clientAuth
EOF_TLS
    openssl x509 -req -in "$tls_dir/$name.csr" -CA "$tls_dir/ca.pem" -CAkey "$tls_dir/ca-key.pem" -CAcreateserial -out "$tls_dir/$name.pem" -days 3650 -sha256 -extfile "$tls_dir/$name.ext" >/dev/null 2>&1
    rm -f "$tls_dir/$name.csr" "$tls_dir/$name.ext"
  done
  chmod 0600 "$tls_dir"/*-key.pem
  chmod 0644 "$tls_dir"/*.pem
  echo "created local TLS bootstrap files: $tls_dir"
}

bootstrap_tls_once
write_config_once "$PACKAGE_DIR/templates/meta.toml" "$CONFIG_DIR/meta.toml"
write_config_once "$PACKAGE_DIR/templates/node.toml" "$CONFIG_DIR/node.toml"
if [ ! -e "$CONFIG_DIR/node-dfs.toml" ]; then
  ln -s "node.toml" "$CONFIG_DIR/node-dfs.toml"
  echo "created compatibility config: $CONFIG_DIR/node-dfs.toml -> node.toml"
fi
if [ ! -e "$CONFIG_DIR/node-ownerfs.toml" ]; then
  ln -s "node.toml" "$CONFIG_DIR/node-ownerfs.toml"
  echo "created compatibility config: $CONFIG_DIR/node-ownerfs.toml -> node.toml"
fi

cat >"$CONFIG_DIR/env" <<EOF
AFS_PREFIX="$PREFIX"
AFS_CONFIG_DIR="$CONFIG_DIR"
AFS_STATE_DIR="$STATE_DIR"
AFS_RUN_DIR="$RUN_DIR"
AFS_LOG_DIR="$LOG_DIR"
AFS_MOUNT_ROOT="$MOUNT_ROOT"
EOF
chmod 0644 "$CONFIG_DIR/env"

echo "installed AFS package under $PREFIX"
echo "config: $CONFIG_DIR"
echo "state:  $STATE_DIR"
echo "mount:  $MOUNT_ROOT"

if [ -n "$START_SERVICE" ]; then
  "$PREFIX/bin/afs-processctl" --prefix "$PREFIX" --config-dir "$CONFIG_DIR" --run-dir "$RUN_DIR" --log-dir "$LOG_DIR" start "$START_SERVICE"
fi
