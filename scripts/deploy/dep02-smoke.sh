#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: dep02-smoke.sh --mount DIR [--name NAME]

Minimal deployment smoke for DEP-01/DEP-02 development feedback. It verifies
that an exact AFS FUSE mount can create, write with fsync, close, reopen and
read back exact content. DFS writes one root-level file. OwnerFs first creates
one workspace directory, then writes one file inside it because the OwnerFs
mount root is a workspace namespace. It is not a full DEP acceptance pass.
Cleanup is best-effort because early slices may not yet implement unlink/rmdir.
USAGE
}

MOUNT=
NAME=afs-dep02-smoke

while [ "$#" -gt 0 ]; do
  case "$1" in
    --mount) MOUNT=${2:-}; shift 2 ;;
    --name) NAME=${2:-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[ -n "$MOUNT" ] || { usage >&2; exit 2; }
[ -d "$MOUNT" ] || { echo "mount directory missing: $MOUNT" >&2; exit 1; }

require_afs_mount() {
  mount=$1
  command -v findmnt >/dev/null 2>&1 || {
    echo "findmnt is required to verify AFS FUSE mounts" >&2
    return 1
  }
  target=$(findmnt -rn --mountpoint "$mount" -o TARGET 2>/dev/null || true)
  fstype=$(findmnt -rn --mountpoint "$mount" -o FSTYPE 2>/dev/null || true)
  source=$(findmnt -rn --mountpoint "$mount" -o SOURCE 2>/dev/null || true)
  if [ -z "$target" ]; then
    echo "not an exact mount point: $mount" >&2
    return 1
  fi
  real_mount=$(readlink -f "$mount")
  real_target=$(readlink -f "$target" 2>/dev/null || true)
  if [ -z "$real_target" ] || [ "$real_target" != "$real_mount" ]; then
    echo "mount point mismatch for $mount: found $target" >&2
    return 1
  fi
  case "$fstype" in
    fuse|fuse.*) ;;
    *) echo "not a FUSE filesystem at $mount: fstype=$fstype" >&2; return 1 ;;
  esac
  case "$source" in
    afs-dfs|afs-ownerfs) ;;
    *) echo "not an AFS DFS/OwnerFs mount at $mount: source=$source" >&2; return 1 ;;
  esac
  AFS_MOUNT_SOURCE=$source
}

AFS_MOUNT_SOURCE=
require_afs_mount "$MOUNT"

case "$NAME" in
  ""|.*|*/*|*..*|*[!A-Za-z0-9._-]*)
    echo "invalid smoke name: $NAME" >&2
    exit 2
    ;;
esac

payload="afs deployment smoke $NAME"
workspace=
case "$AFS_MOUNT_SOURCE" in
  afs-ownerfs)
    workspace="$MOUNT/afs-smoke-$NAME-$$"
    file="$workspace/payload.txt"
    [ ! -e "$workspace" ] || { echo "smoke workspace already exists: $workspace" >&2; exit 1; }
    mkdir "$workspace"
    ;;
  afs-dfs)
    file="$MOUNT/afs-smoke-$NAME-$$.txt"
    [ ! -e "$file" ] || { echo "smoke file already exists: $file" >&2; exit 1; }
    ;;
  *)
    echo "unsupported AFS mount source: $AFS_MOUNT_SOURCE" >&2
    exit 1
    ;;
esac

printf '%s\n' "$payload" | dd of="$file" bs=1 conv=fsync status=none
read_back=$(cat "$file")
[ "$read_back" = "$payload" ] || {
  echo "payload mismatch after reopen" >&2
  exit 1
}
cleanup=removed
if ! rm -f "$file" 2>/dev/null; then
  cleanup=left-in-place
fi
if [ -n "$workspace" ] && ! rmdir "$workspace" 2>/dev/null; then
  cleanup=left-in-place
fi
if [ "$cleanup" = removed ]; then
  echo "DEP smoke ok: $MOUNT source=$AFS_MOUNT_SOURCE cleanup=removed"
else
  echo "DEP smoke ok: $MOUNT source=$AFS_MOUNT_SOURCE cleanup=left-in-place path=${workspace:-$file}" >&2
  echo "DEP smoke ok: $MOUNT source=$AFS_MOUNT_SOURCE cleanup=left-in-place"
fi
