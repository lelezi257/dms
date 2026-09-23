#!/usr/bin/env bash
set -euo pipefail
base="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
config="${DMS_HOME_CONFIG:-$base/config/homefs.env}"
[[ -f "$config" ]] || { echo "missing config: $config" >&2; exit 2; }
# shellcheck source=/dev/null
source "$config"
case "${1:-}" in
  center)
    export DMS_HOME_TOKEN
    exec "$base/bin/dms-home" center "$DMS_HOME_CENTER_RPC" "$DMS_HOME_CENTER_HTTP" "$DMS_HOME_CENTER_STATE" ;;
  node)
    export DMS_HOME_TOKEN
    exec "$base/bin/dms-home" node "$DMS_HOME_NODE_ID" "$DMS_HOME_CENTER_RPC" \
      "$DMS_HOME_NFS_ENDPOINT" "$DMS_HOME_P2P_ADDRESS" "$DMS_HOME_DATA_ROOT" \
      "$DMS_HOME_PEER_MOUNTS" "$DMS_HOME_FUSE_MOUNT" "$DMS_HOME_BACKEND" ;;
  locate)
    exec "$base/bin/dms-home" locate "${2:?root name required}" "$DMS_HOME_CENTER_RPC" ;;
  roots)
    exec "$base/bin/dms-home" roots "$DMS_HOME_CENTER_RPC" ;;
  *) echo "usage: $0 center|node|locate ROOT|roots" >&2; exit 2 ;;
esac
