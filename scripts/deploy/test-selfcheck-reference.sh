#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
SELF_CHECK="$SCRIPT_DIR/afs-selfcheck"
TMP_DIR=${TMPDIR:-/tmp}/afs-selfcheck-reference-test-$$
mkdir -p "$TMP_DIR"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

python3 - "$SELF_CHECK" <<'PY'
import importlib.machinery
import importlib.util
import pathlib
import sys

script = pathlib.Path(sys.argv[1])
loader = importlib.machinery.SourceFileLoader("afs_selfcheck", str(script))
spec = importlib.util.spec_from_loader(loader.name, loader)
module = importlib.util.module_from_spec(spec)
sys.modules[loader.name] = module
loader.exec_module(module)

size = 8 * module.MiB
case_id = "selfcheck-reference-regression"
base_seed = f"{case_id}:base:{size}"
patches = module.default_patches(size, case_id)

buf = bytearray()
offset = 0
while offset < size:
    want = min(module.STREAM_BLOCK, size - offset)
    buf.extend(module.pattern(base_seed, offset, want))
    offset += want

for patch in patches:
    done = 0
    remaining = patch.length
    while remaining:
        want = min(module.STREAM_BLOCK, remaining)
        buf[patch.offset + done : patch.offset + done + want] = module.pattern(patch.seed, done, want)
        done += want
        remaining -= want

offset = 0
while offset < size:
    want = min(module.STREAM_BLOCK, size - offset)
    expected = module.overlay_block(base_seed, patches, offset, want)
    actual = bytes(buf[offset : offset + want])
    if actual != expected:
        raise AssertionError(f"overlay mismatch at {offset}")
    offset += want

unaligned = patches[0]
start = module.STREAM_BLOCK * 2
want = module.STREAM_BLOCK
assert start < unaligned.offset < start + want
print("ok - selfcheck overlay matches unaligned patch write chunks")
PY
