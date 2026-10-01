"""Verify portable evidence hashes and Markdown links without build or services."""
import hashlib
import json
import pathlib
import platform
import re
import sys

assert platform.system() == 'Linux'
root = pathlib.Path(sys.argv[1]).resolve()
manifest_path = root / 'artifact-sha256.json'
exclusions = {'artifact-sha256.json', 'portable-audit.json'}
files = sorted(p for p in root.rglob('*') if p.is_file() and p.relative_to(root).as_posix() not in exclusions)
actual = {p.relative_to(root).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest() for p in files}
if '--create' in sys.argv[2:]:
    assert not manifest_path.exists(), 'keep old catalogs immutable'
    manifest_path.write_text(json.dumps(actual, indent=2) + '\n')
assert json.loads(manifest_path.read_text()) == actual, 'evidence hash/catalog mismatch'
links = 0
for page in root.rglob('*.md'):
    for destination in re.findall(r'\[[^\]]*\]\(([^)]+)\)', page.read_text()):
        if destination.startswith(('http:', 'https:', '#')):
            continue
        target = destination.split('#', 1)[0]
        assert (page.parent / target).exists(), (page, destination)
        links += 1
print(json.dumps({'status': 'PASS', 'artifact_hashes': len(actual), 'local_links': links}, indent=2))
