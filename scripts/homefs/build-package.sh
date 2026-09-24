#!/usr/bin/env bash
# Run on Linux from a complete source checkout. Creates a local, unpublished preview archive.
set -euo pipefail

source_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
cd "$source_dir"
[[ "$(uname -s)" == Linux ]] || { echo 'Linux required' >&2; exit 2; }
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$source_dir/target-homefs}"

cargo build -p dms-home --release --locked
binary="$CARGO_TARGET_DIR/release/dms-home"
[[ -x "$binary" ]] || { echo "missing $binary" >&2; exit 2; }

third_party="${DMS_THIRD_PARTY_DIR:-$source_dir/THIRD-PARTY-LICENSES}"
if [[ ! -d "$third_party" ]]; then
  echo "Generate third-party licenses first: python3 scripts/release/dependency_inventory.py --output '$third_party'" >&2
  exit 2
fi

output_root="${1:-$source_dir/artifacts/homefs}"
mkdir -p "$output_root"
output_root="$(cd "$output_root" && pwd -P)"
arch="$(uname -m)"
name="dms-home-preview-0.1.0-linux-$arch"
build_id="${DMS_HOME_BUILD_ID:-$(date -u +%Y%m%dT%H%M%SZ)}"
[[ "$build_id" =~ ^[A-Za-z0-9][A-Za-z0-9._-]*$ && "$build_id" != *..* ]] || { echo 'invalid build id' >&2; exit 2; }
source_revision="${DMS_HOME_SOURCE_REV:-}"
if [[ -z "$source_revision" && -f SOURCE-REVISION ]]; then
  source_revision="$(cat SOURCE-REVISION)"
fi
if [[ -z "$source_revision" ]]; then
  source_revision="$(git rev-parse HEAD 2>/dev/null || true)"
  if [[ -n "$source_revision" && -n "$(git status --porcelain 2>/dev/null)" ]]; then
    echo 'refusing to package a dirty checkout without an explicit source revision' >&2
    exit 2
  fi
fi
[[ "$source_revision" =~ ^[0-9a-f]{40}$ ]] || { echo 'set DMS_HOME_SOURCE_REV to the exact committed source SHA' >&2; exit 2; }
stage="$output_root/$name-$build_id/$name"
[[ ! -e "$stage" ]] || { echo "refusing to overwrite $stage" >&2; exit 2; }
mkdir -p "$stage/bin" "$stage/config" "$stage/scripts" "$stage/docs" "$stage/THIRD-PARTY-LICENSES"
install -m 0755 "$binary" "$stage/bin/dms-home"
install -m 0644 scripts/homefs/homefs.env.example "$stage/config/homefs.env.example"
install -m 0755 scripts/homefs/run.sh "$stage/scripts/run.sh"
install -m 0755 scripts/homefs/setup-nfs.sh "$stage/scripts/setup-nfs.sh"
install -m 0644 docs/agent-home-preview-installation.md "$stage/docs/installation.md"
install -m 0644 LICENSE "$stage/LICENSE"
install -m 0644 NOTICE "$stage/NOTICE"
cp -a "$third_party/." "$stage/THIRD-PARTY-LICENSES/"

(
  cd "$stage"
  printf '%s\n' \
    'package=dms-home-preview' \
    'status=local test candidate; not a release' \
    "arch=$arch" \
    "rust_version=$(rustc --version)" \
    "source_revision=$source_revision" \
    >PACKAGE-METADATA
  find . -type f ! -name SHA256SUMS -printf '%P\n' | LC_ALL=C sort >MANIFEST.txt
  xargs -r sha256sum <MANIFEST.txt >SHA256SUMS
)
archive="$output_root/$name-$build_id/$name.tar.gz"
tar --sort=name --mtime="@${SOURCE_DATE_EPOCH:-0}" --owner=0 --group=0 --numeric-owner \
  -C "$(dirname "$stage")" -cf - "$name" | gzip -n >"$archive"
(cd "$(dirname "$archive")" && sha256sum "$(basename "$archive")" >"$(basename "$archive").sha256")
echo "$archive"
