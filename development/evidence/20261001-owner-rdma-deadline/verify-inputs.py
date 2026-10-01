#!/usr/bin/env python3
"""Linux-only identity/link audit; it does not run release acceptance."""
import hashlib
import json
import pathlib
import re
import sys

assert sys.platform == "linux", "Run audits in Linux"
source, previous, executed = map(pathlib.Path, sys.argv[1:4])
old = json.loads(previous.read_text())["files"]

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

changed = [name for name, value in old.items() if digest(source / name) != value]
assert changed == ["tests/ownerfs_peer_contract.rs"], changed
assert digest(source / changed[0]) == digest(executed / changed[0])
assert digest(source / "scripts/check-rdma-cancellation.py") == digest(executed / "scripts/check-rdma-cancellation.py")
assert digest(source / "AGENTS.md") == "539ff7119bf3cde494478921db7967e8c152c8ef64c9c43c3ee3a64e1bdb7f6f"
assert digest(source / "docs/handoff.md") == "8941901e6b6f8b8b10e057a7477695dfa9b1fc4876b5762e71b66d8930eb7216"
pages = [source / "development" / name for name in ("implementation.md", "validation.md", "plan.md", "issues.md")]
pages.append(pathlib.Path(__file__).parent / "README.md")
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
            assert anchor in anchors, (page, target, anchors)
        links.append({"page": str(page.relative_to(source)), "target": target})
directory = pathlib.Path(__file__).parent
artifacts = {str(p.relative_to(directory)): digest(p) for p in sorted(directory.rglob("*")) if p.is_file() and p.name != "input-audit.json" and "__pycache__" not in p.parts}
result = {"status": "PASS", "level": "local identity/document audit", "previous_input_count": len(old), "changed_inputs": changed, "production_inputs_unchanged": len(old) - 1, "executed_test_and_checker_match": True, "protected_documents_unchanged": True, "links": links, "artifacts": artifacts, "formal_acceptance": "NOT_RUN", "full_candidate_source_gate": "DUE_AT_BATCH_END"}
print(json.dumps(result, indent=2))
