#!/usr/bin/env bash
set -euo pipefail

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
TMP=${TMPDIR:-/tmp}/afs-package-repro-$$
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

mkdir -p "$TMP/bin" "$TMP/out-a" "$TMP/out-b"
cat >"$TMP/bin/afs-meta" <<'EOF_META'
#!/usr/bin/env bash
printf 'afs-meta 0.1.0\n'
EOF_META
cat >"$TMP/bin/afs-node" <<'EOF_NODE'
#!/usr/bin/env bash
printf 'afs-node 0.1.0\n'
EOF_NODE
chmod 0755 "$TMP/bin/afs-meta" "$TMP/bin/afs-node"

SOURCE_DATE_EPOCH=1234567890 "$ROOT/build-package.sh" \
  --bin-dir "$TMP/bin" \
  --output "$TMP/out-a" \
  --version 0.1.0 \
  --source-commit b259c44f82be90ae07158501295ddbc5359e7a35 \
  --features test-feature \
  >"$TMP/package-a.path"
sleep 2
old_umask=$(umask)
umask 077
SOURCE_DATE_EPOCH=1234567890 "$ROOT/build-package.sh" \
  --bin-dir "$TMP/bin" \
  --output "$TMP/out-b" \
  --version 0.1.0 \
  --source-commit b259c44f82be90ae07158501295ddbc5359e7a35 \
  --features test-feature \
  >"$TMP/package-b.path"
umask "$old_umask"

pkg_a=$(cat "$TMP/package-a.path")
pkg_b=$(cat "$TMP/package-b.path")
if ! cmp -s "$pkg_a" "$pkg_b"; then
  sha256sum "$pkg_a" "$pkg_b" >&2
  exit 1
fi
if ! cmp -s "$pkg_a.sha256" "$pkg_b.sha256"; then
  cat "$pkg_a.sha256" "$pkg_b.sha256" >&2
  exit 1
fi

tar -xzf "$pkg_a" -C "$TMP"
manifest="$TMP/afs-0.1.0-linux-$(uname -m)/manifest.json"
grep -q '"source_date_epoch": 1234567890' "$manifest"
python3 -m json.tool "$manifest" >/dev/null
package_dir=$(dirname "$manifest")
cmp "$ROOT/../../docs/deployment/trial.md" "$package_dir/docs/deployment/trial.md"
test ! -e "$package_dir/bin/afs-workspace-probe"
python3 - "$manifest" <<'PY_MANIFEST'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as stream:
    manifest = json.load(stream)
assert "docs/deployment/trial.md" in manifest["contains"]
assert "docs/guides/trial.md" not in manifest["contains"]
PY_MANIFEST
tar -tzvf "$pkg_b" | grep '^drwxr-xr-x 0/0 .* afs-0.1.0-linux-'"$(uname -m)"'/$' >/dev/null
(cd "$TMP/out-a" && sha256sum -c "afs-0.1.0-linux-$(uname -m).tar.gz.sha256") >/dev/null

if SOURCE_DATE_EPOCH=1234567890 "$ROOT/build-package.sh" \
  --bin-dir "$TMP/bin" \
  --output "$TMP/out-a" \
  --version 0.1.0 \
  --source-commit b259c44f82be90ae07158501295ddbc5359e7a35 \
  --features 'bad"json' \
  --force \
  >"$TMP/bad-feature.path" 2>"$TMP/bad-feature.err"; then
  echo "unsafe feature list unexpectedly packaged" >&2
  exit 1
fi
grep -q 'invalid features list' "$TMP/bad-feature.err"

printf 'ok - build-package creates byte-identical archives from identical inputs\n'
