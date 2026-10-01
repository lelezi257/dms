"""Linux read-only check of the portable evidence catalog and local links."""
import hashlib
import json
import pathlib
import platform
import re
import sys

assert platform.system() == "Linux"
root = pathlib.Path(sys.argv[1]).resolve()
listed = {}
for line in (root / "artifact-hashes.txt").read_text().splitlines():
    digest, name = line.split("  ", 1)
    assert name not in listed, name
    listed[name] = digest
    assert hashlib.sha256((root / name).read_bytes()).hexdigest() == digest, name
actual = {str(p.relative_to(root)) for p in root.rglob("*") if p.is_file()
          and p.name not in ("artifact-hashes.txt", "portable-audit.json")}
assert set(listed) == actual, (set(listed) ^ actual)
links = 0
for page in root.glob("*.md"):
    for target in re.findall(r"\[[^\]]*\]\(([^)]+)\)", page.read_text()):
        if "://" in target or target.startswith("#"):
            continue
        assert (page.parent / target.split("#", 1)[0]).is_file(), (page, target)
        links += 1
print(json.dumps({"status": "PASS", "artifact_hashes": len(listed), "local_links": links}, indent=2))
