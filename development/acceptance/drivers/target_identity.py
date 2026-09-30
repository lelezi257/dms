"""Fail-closed suite target checks; labels cannot substitute for observed identity."""
from __future__ import annotations

import json
from pathlib import Path
from typing import Any


def mount_record(command: dict[str, Any]) -> dict[str, Any]:
    if command.get("returncode") != 0:
        return {}
    try:
        records = json.loads(command.get("stdout", "")).get("filesystems", [])
        return records[0] if len(records) == 1 else {}
    except (ValueError, TypeError, AttributeError):
        return {}


def process_matches(identity: dict[str, Any] | None, executable: str) -> bool:
    if not identity or identity.get("exists") is not True:
        return False
    digest = identity.get("exe_sha256")
    return (
        Path(str(identity.get("exe", ""))).name == executable
        and isinstance(digest, str)
        and len(digest) == 64
        and all(char in "0123456789abcdef" for char in digest)
        and bool(identity.get("cmdline"))
    )


def target_checks(
    system: str,
    backend: str | None,
    mount: dict[str, Any],
    base_mount: dict[str, Any],
    node: dict[str, Any] | None,
    meta: dict[str, Any] | None,
) -> dict[str, bool]:
    record = mount_record(mount)
    base = mount_record(base_mount)
    label = str(backend or "").lower()
    expected = {"ownerfs": "afs-ownerfs", "dfs": "afs-dfs"}.get(label)
    reference = label in {"reference", "ext4"}
    target_ok = (
        record.get("fstype") == "ext4"
        if reference
        else expected is not None
        and str(record.get("fstype", "")).startswith("fuse")
        and record.get("source") == expected
    )
    return {
        "linux-runtime": system == "Linux",
        "observed-target-backend": bool(record) and bool(target_ok),
        "same-fixture-filesystem": bool(base) and all(
            base.get(key) == record.get(key) for key in ("target", "source", "fstype")
        ),
        "product-process-identity": reference
        or (process_matches(node, "afs-node") and process_matches(meta, "afs-meta")),
    }
