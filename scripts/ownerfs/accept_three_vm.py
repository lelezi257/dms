#!/usr/bin/env python3
"""Compatibility entrypoint for OwnerFs three-VM acceptance."""

from pathlib import Path
import runpy
import sys

SOURCE_ROOT = Path(__file__).resolve().parents[2]
TARGET = SOURCE_ROOT / "tests" / "ownerfs_acceptance.py"

if __name__ == "__main__":
    sys.argv[0] = str(TARGET)
    runpy.run_path(str(TARGET), run_name="__main__")
