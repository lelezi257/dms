#!/usr/bin/env bash
# Run on a trusted Linux test node as root. Exports only the configured home data root.
set -euo pipefail
[[ "$(id -u)" == 0 ]] || { echo 'run as root' >&2; exit 2; }
data_root="${1:?data root required}"
client_cidr="${2:?trusted client CIDR required}"
[[ "$data_root" == /* && "$data_root" != *$'\n'* && "$client_cidr" != *$'\n'* ]] || exit 2
install -d -m 0750 "$data_root" /etc/exports.d
export_file=/etc/exports.d/dms-home-preview.exports
printf '%s %s(rw,sync,fsid=0,no_subtree_check,no_root_squash)\n' \
  "$data_root" "$client_cidr" >"$export_file"
systemctl enable --now nfs-server
exportfs -ra
exportfs -v
