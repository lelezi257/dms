#!/usr/bin/env python3
"""Bind recorded source gate and documents to the current candidate on Linux."""
import hashlib
import json
import pathlib
import re
import sys

assert sys.platform == "linux"
source, executed = map(pathlib.Path, sys.argv[1:3])
old = json.loads((source / "development/evidence/20261001-posted-rdma-cancellation/posted-cancel-v76-r2/build-inputs.json").read_text())["files"]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


current = {name: digest(source / name) for name in old}
assert current == {name: digest(executed / name) for name in old}, "executed compiler inputs differ"
changed = [name for name in old if current[name] != old[name]]
assert changed == ["tests/ownerfs_peer_contract.rs"], changed
assert digest(source / "AGENTS.md") == "539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f"
assert digest(source / "docs/handoff.md") == "8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216"
assert digest(source / "scripts/deploy/afs-processctl") == "01f38d32f34994f89bea3c75f57eecc506db6157309e74f3fe5425f64775601e"
for name in ("scripts/deploy/selftest.sh", "scripts/deploy/afs-processctl", "scripts/check-rdma-cancellation.py"):
    assert digest(source / name) == digest(executed / name), name
pages = [source / "development" / name for name in ("implementation.md", "validation.md", "plan.md", "issues.md")]
pages += [source / "docs/status.md", source / "development/evidence/20261001-owner-rdma-deadline/README.md", pathlib.Path(__file__).parent / "README.md"]
links = []
for page in pages:
    for target in re.findall(r"\]\(([^)]+)\)", page.read_text()):
        if "://" in target:
            continue
        path, _, anchor = target.partition("#")
        dest = (page.parent / path).resolve() if path else page
        assert dest.is_file(), (page, target)
        if anchor:
            headings = re.findall(r"^#+\s+(.*)$", dest.read_text(), re.M)
            anchors = {re.sub(r"[^\w\- ]", "", h.lower()).replace(" ", "-") for h in headings}
            assert anchor in anchors, (page, target)
        links.append({"page": str(page.relative_to(source)), "target": target})
print(json.dumps({"status": "PASS", "level": "source/document identity audit", "compiler_inputs": current, "changed_inputs": changed, "unchanged_production_inputs": len(old) - 1, "links": links, "protected_documents_unchanged": True, "formal_acceptance": "NOT_RUN"}, indent=2))
