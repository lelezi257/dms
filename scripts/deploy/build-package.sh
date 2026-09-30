#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: build-package.sh --bin-dir DIR --output DIR [--version VERSION] [--package-name NAME] [--source-commit SHA] [--features LIST] [--force]

Create an AFS release package from already-built Linux binaries. This script
does not run cargo and does not install dependencies.
USAGE
}

BIN_DIR=
OUTPUT_DIR=
VERSION=
PACKAGE_NAME=afs
SOURCE_COMMIT=
FEATURES=${AFS_PACKAGE_FEATURES:-unknown}
FORCE=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --bin-dir)
      BIN_DIR=${2:-}
      shift 2
      ;;
    --output)
      OUTPUT_DIR=${2:-}
      shift 2
      ;;
    --version)
      VERSION=${2:-}
      shift 2
      ;;
    --package-name)
      PACKAGE_NAME=${2:-}
      shift 2
      ;;
    --source-commit)
      SOURCE_COMMIT=${2:-}
      shift 2
      ;;
    --features)
      FEATURES=${2:-}
      shift 2
      ;;
    --force)
      FORCE=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ -z "$BIN_DIR" ] || [ -z "$OUTPUT_DIR" ]; then
  usage >&2
  exit 2
fi

safe_name() {
  case "$1" in
    ""|.*|*/*|*..*|*[!A-Za-z0-9._+-]*)
      return 1
      ;;
  esac
}

safe_name "$PACKAGE_NAME" || { echo "invalid package name: $PACKAGE_NAME" >&2; exit 2; }

if [ ! -x "$BIN_DIR/afs-meta" ] || [ ! -x "$BIN_DIR/afs-node" ]; then
  echo "missing executable afs-meta or afs-node in $BIN_DIR" >&2
  exit 1
fi

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
SOURCE_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)
if [ -z "$VERSION" ]; then
  VERSION=$("$BIN_DIR/afs-meta" --version 2>/dev/null | awk '{print $NF; exit}')
fi
safe_name "$VERSION" || { echo "invalid version: $VERSION" >&2; exit 2; }

ARCH=${AFS_PACKAGE_ARCH:-$(uname -m)}
TARGET=${AFS_PACKAGE_TARGET:-linux-$ARCH}
safe_name "$TARGET" || { echo "invalid target: $TARGET" >&2; exit 2; }

mkdir -p "$OUTPUT_DIR"
OUTPUT_DIR=$(CDPATH= cd -- "$OUTPUT_DIR" && pwd)
STAGING=$(mktemp -d "$OUTPUT_DIR/.afs-package.XXXXXX")
cleanup() {
  rm -rf "$STAGING"
}
trap cleanup EXIT
PACKAGE_DIR="$STAGING/$PACKAGE_NAME-$VERSION-$TARGET"
TARBALL="$OUTPUT_DIR/$PACKAGE_NAME-$VERSION-$TARGET.tar.gz"
if [ -e "$TARBALL" ] && [ "$FORCE" -ne 1 ]; then
  echo "refusing to overwrite existing package: $TARBALL" >&2
  exit 1
fi

mkdir -p "$PACKAGE_DIR/bin" "$PACKAGE_DIR/scripts" "$PACKAGE_DIR/templates"
install -m 0755 "$BIN_DIR/afs-meta" "$PACKAGE_DIR/bin/afs-meta"
install -m 0755 "$BIN_DIR/afs-node" "$PACKAGE_DIR/bin/afs-node"
install -m 0755 "$SCRIPT_DIR/install.sh" "$PACKAGE_DIR/install.sh"
install -m 0755 "$SCRIPT_DIR/afs-processctl" "$PACKAGE_DIR/bin/afs-processctl"
install -m 0755 "$SCRIPT_DIR/dep02-smoke.sh" "$PACKAGE_DIR/bin/dep02-smoke.sh"
install -m 0644 "$SCRIPT_DIR/DEPENDENCIES.md" "$PACKAGE_DIR/DEPENDENCIES.md"
find "$SCRIPT_DIR/templates" -type f ! -name '._*' -print | while IFS= read -r template; do
  rel=${template#"$SCRIPT_DIR/templates/"}
  mkdir -p "$PACKAGE_DIR/templates/$(dirname "$rel")"
  install -m 0644 "$template" "$PACKAGE_DIR/templates/$rel"
done

if [ -z "$SOURCE_COMMIT" ] && command -v git >/dev/null 2>&1 && git -C "$SOURCE_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  SOURCE_COMMIT=$(git -C "$SOURCE_ROOT" rev-parse HEAD)
fi
if [ -z "$SOURCE_COMMIT" ] || ! printf '%s' "$SOURCE_COMMIT" | grep -Eq '^[0-9a-fA-F]{7,64}$'; then
  echo "source commit is required; pass --source-commit when packaging outside a git checkout" >&2
  exit 1
fi

TOOLCHAIN=unknown
if [ -f "$SOURCE_ROOT/rust-toolchain.toml" ]; then
  TOOLCHAIN=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$SOURCE_ROOT/rust-toolchain.toml" | head -n 1)
fi

meta_sha=$(sha256sum "$BIN_DIR/afs-meta" | awk '{print $1}')
node_sha=$(sha256sum "$BIN_DIR/afs-node" | awk '{print $1}')
{
  echo "# ldd afs-meta"
  if command -v ldd >/dev/null 2>&1; then ldd "$BIN_DIR/afs-meta" || true; else echo "ldd unavailable"; fi
  echo "# ldd afs-node"
  if command -v ldd >/dev/null 2>&1; then ldd "$BIN_DIR/afs-node" || true; else echo "ldd unavailable"; fi
} >"$PACKAGE_DIR/ldd.txt"

cat >"$PACKAGE_DIR/manifest.json" <<EOF
{
  "schema": "afs.release-package.v1",
  "name": "$PACKAGE_NAME",
  "version": "$VERSION",
  "target": "$TARGET",
  "source_commit": "$SOURCE_COMMIT",
  "features": "$FEATURES",
  "rust_toolchain": "$TOOLCHAIN",
  "binaries": {
    "afs-meta": {"sha256": "$meta_sha"},
    "afs-node": {"sha256": "$node_sha"}
  },
  "contains": ["afs-meta", "afs-node", "afs-processctl", "dep02-smoke.sh"],
  "installer": "install.sh",
  "notes": "Built from existing Linux release binaries; no Cargo or Git required on target guests."
}
EOF

(cd "$PACKAGE_DIR" && find . -type f ! -name SHA256SUMS -print | sort | xargs sha256sum > SHA256SUMS)
if command -v xattr >/dev/null 2>&1; then
  xattr -cr "$PACKAGE_DIR" 2>/dev/null || true
fi
if [ -e "$TARBALL" ]; then
  rm -f "$TARBALL" "$TARBALL.sha256"
fi
TAR_CREATE_ARGS=()
for opt in --no-xattrs --no-acls --disable-copyfile --no-fflags; do
  if tar "$opt" -cf /dev/null --files-from /dev/null >/dev/null 2>&1; then
    TAR_CREATE_ARGS+=("$opt")
  fi
done
(cd "$STAGING" && COPYFILE_DISABLE=1 tar "${TAR_CREATE_ARGS[@]}" -czf "$TARBALL" "$(basename "$PACKAGE_DIR")")
sha256sum "$TARBALL" >"$TARBALL.sha256"

echo "$TARBALL"
