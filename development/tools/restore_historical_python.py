#!/usr/bin/env python3
"""Restore removed historical Python evidence files from an immutable Git map."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
from pathlib import Path
from typing import Any


HEX40 = re.compile(r"^[0-9a-f]{40}$")
HEX64 = re.compile(r"^[0-9a-f]{64}$")


class RestoreError(ValueError):
    pass


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def load_entries(manifest_path: Path) -> list[dict[str, Any]]:
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    entries = manifest.get("files")
    if not isinstance(entries, list):
        raise RestoreError("manifest.files must be a list")
    return entries


def validate_rel_path(value: Any) -> str:
    if not isinstance(value, str) or not value:
        raise RestoreError("path must be a non-empty string")
    rel = Path(value)
    if rel.is_absolute() or ".." in rel.parts:
        raise RestoreError(f"path escapes restore root: {value}")
    if any(part in {"", "."} for part in rel.parts):
        raise RestoreError(f"path is not normalized: {value}")
    return value


def validate_entry(entry: dict[str, Any]) -> tuple[str, str, str, str, int, int]:
    commit = entry.get("commit")
    path = validate_rel_path(entry.get("path"))
    blob = entry.get("git_blob")
    sha256 = entry.get("sha256")
    size = entry.get("bytes")
    mode = entry.get("mode")
    if not isinstance(commit, str) or not HEX40.fullmatch(commit):
        raise RestoreError(f"{path}: commit must be a 40-hex object id")
    if not isinstance(blob, str) or not HEX40.fullmatch(blob):
        raise RestoreError(f"{path}: git_blob must be a 40-hex object id")
    if not isinstance(sha256, str) or not HEX64.fullmatch(sha256):
        raise RestoreError(f"{path}: sha256 must be a 64-hex digest")
    if not isinstance(size, int) or size < 0:
        raise RestoreError(f"{path}: bytes must be a non-negative integer")
    if not isinstance(mode, str) or not re.fullmatch(r"100[0-7]{3}", mode):
        raise RestoreError(f"{path}: mode must be a regular-file Git mode")
    return commit, path, blob, sha256, size, int(mode, 8)


def git_tree_entry(repo: Path, commit: str, rel: str) -> tuple[str, str]:
    proc = subprocess.run(
        ["git", "-C", str(repo), "ls-tree", commit, "--", rel],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if proc.returncode != 0:
        raise RestoreError(f"{rel}: cannot read git tree {commit}: {proc.stderr.strip()}")
    line = proc.stdout.strip()
    if not line:
        raise RestoreError(f"{rel}: path missing from git tree {commit}")
    mode, kind, blob, path = line.split(None, 3)
    if kind != "blob" or path != rel:
        raise RestoreError(f"{rel}: git tree entry is not the expected blob path")
    return mode, blob


def git_blob(repo: Path, commit: str, rel: str, blob: str, mode: int) -> bytes:
    tree_mode, tree_blob = git_tree_entry(repo, commit, rel)
    if tree_blob != blob:
        raise RestoreError(f"{rel}: git tree blob mismatch: {tree_blob}")
    if int(tree_mode, 8) != mode:
        raise RestoreError(f"{rel}: git tree mode mismatch: {tree_mode}")
    proc = subprocess.run(
        ["git", "-C", str(repo), "cat-file", "-p", blob],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if proc.returncode != 0:
        raise RestoreError(f"missing git blob {blob}: {proc.stderr.decode('utf-8', 'replace').strip()}")
    return proc.stdout


def prepare_output_root(output_root: Path) -> Path:
    if output_root.exists() or output_root.is_symlink():
        if output_root.is_symlink():
            raise RestoreError(f"output root is a symlink: {output_root}")
        if not output_root.is_dir():
            raise RestoreError(f"output root is not a directory: {output_root}")
    else:
        output_root.mkdir(parents=True)
    return output_root.resolve()


def ensure_safe_parent(output_root: Path, root_real: Path, rel: str) -> Path:
    current = output_root
    parts = Path(rel).parts[:-1]
    for part in parts:
        current = current / part
        if current.is_symlink():
            raise RestoreError(f"{rel}: output parent is a symlink: {current}")
        if current.exists():
            if not current.is_dir():
                raise RestoreError(f"{rel}: output parent is not a directory: {current}")
        else:
            current.mkdir()
        current_real = current.resolve()
        if os.path.commonpath([str(root_real), str(current_real)]) != str(root_real):
            raise RestoreError(f"path escapes restore root: {rel}")
    return current


def restore(manifest_path: Path, repo: Path, output_root: Path) -> dict[str, int]:
    root_real = prepare_output_root(output_root)
    seen: set[str] = set()
    restored = 0
    total_bytes = 0
    for entry in load_entries(manifest_path):
        if not isinstance(entry, dict):
            raise RestoreError("manifest entries must be objects")
        commit, rel, blob_id, expected_sha, expected_size, mode = validate_entry(entry)
        if rel in seen:
            raise RestoreError(f"duplicate path in manifest: {rel}")
        seen.add(rel)
        data = git_blob(repo, commit, rel, blob_id, mode)
        actual_sha = sha256_bytes(data)
        if actual_sha != expected_sha:
            raise RestoreError(f"{rel}: sha256 mismatch for {blob_id}: {actual_sha}")
        if len(data) != expected_size:
            raise RestoreError(f"{rel}: byte count mismatch for {blob_id}: {len(data)}")
        dest = (output_root / rel)
        ensure_safe_parent(output_root, root_real, rel)
        if dest.exists() or dest.is_symlink():
            raise RestoreError(f"{rel}: output already exists")
        dest.write_bytes(data)
        os.chmod(dest, stat.S_IMODE(mode))
        restored += 1
        total_bytes += len(data)
    return {"files": restored, "bytes": total_bytes}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--repo", default=Path.cwd(), type=Path)
    parser.add_argument("--output-root", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        result = restore(args.manifest, args.repo, args.output_root)
    except RestoreError as exc:
        print(f"restore failed: {exc}", file=sys.stderr)
        return 2
    print(json.dumps({"status": "PASS", **result}, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
