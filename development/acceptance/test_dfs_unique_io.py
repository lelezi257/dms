"""Linux qualification tests for the fixed dfs_unique_io C probe."""
from __future__ import annotations

import hashlib
import json
import os
import platform
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


DATASET = "counter-1m-v1"
DATA_BYTES = 64 * 1024 * 1024
BLOCK_BYTES = 1024 * 1024
OPERATIONS = DATA_BYTES // BLOCK_BYTES
PATTERN_BYTE = 0x61
CHUNK_BYTES = 4 * 1024 * 1024


def expected_block(index: int) -> bytes:
    block = bytearray([PATTERN_BYTE]) * BLOCK_BYTES
    block[:8] = index.to_bytes(8, "little")
    return bytes(block)


def expected_payload_sha256() -> str:
    digest = hashlib.sha256()
    for index in range(OPERATIONS):
        digest.update(expected_block(index))
    return digest.hexdigest()


EXPECTED_SHA256 = expected_payload_sha256()


def compile_probe(root: Path) -> Path:
    source = Path(__file__).parent / "probes" / "dfs_unique_io.c"
    binary = root / "dfs_unique_io"
    command = ["cc", "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror", str(source), "-o", str(binary)]
    subprocess.run(command, check=True, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return binary


def parse_success(completed: subprocess.CompletedProcess[str]) -> dict[str, object]:
    if completed.returncode != 0:
        raise AssertionError(f"probe failed rc={completed.returncode} stderr={completed.stderr!r}")
    if completed.stderr:
        raise AssertionError(f"successful probe wrote stderr: {completed.stderr!r}")
    lines = completed.stdout.splitlines()
    if len(lines) != 1:
        raise AssertionError(f"expected one JSON stdout line, got {len(lines)}")
    return json.loads(lines[0])


def validate_io_result(record: dict[str, object], operation: str, barrier: str) -> None:
    expected: dict[str, object] = {
        "dataset": DATASET,
        "operation": operation,
        "file_bytes": DATA_BYTES,
        "io_bytes": DATA_BYTES,
        "block_bytes": BLOCK_BYTES,
        "concurrency": 1,
        "pattern_byte": PATTERN_BYTE,
        "operations": OPERATIONS,
        "barrier": barrier,
        "cache_requested": "unobserved",
        "residency_observed": False,
        "content_ok": True,
    }
    for key, value in expected.items():
        if record.get(key) != value:
            raise AssertionError(f"unexpected {key}: {record.get(key)!r}")
    for key in ("wall_ns", "client_cpu_ns", "barrier_ns"):
        if not isinstance(record.get(key), int) or record[key] <= 0:
            raise AssertionError(f"expected positive integer {key}: {record.get(key)!r}")


@unittest.skipUnless(platform.system() == "Linux", "dfs_unique_io qualification is Linux-only")
class DfsUniqueIoQualificationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        if shutil.which("cc") is None:
            raise unittest.SkipTest("cc is required")
        cls.tempdir = tempfile.TemporaryDirectory(prefix="dfs-unique-io-", dir="/var/tmp")
        cls.root = Path(cls.tempdir.name)
        cls.probe = compile_probe(cls.root)

    @classmethod
    def tearDownClass(cls) -> None:
        cls.tempdir.cleanup()

    def run_probe(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run([str(self.probe), *args], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def test_write_and_read_emit_compatible_shape_and_expected_unique_content(self) -> None:
        payload = self.root / "payload.bin"
        write_record = parse_success(self.run_probe("write", str(payload)))
        validate_io_result(write_record, "seq-write", "fdatasync")
        read_record = parse_success(self.run_probe("read", str(payload)))
        validate_io_result(read_record, "seq-read", "close")

        digest = hashlib.sha256()
        chunk_digests: list[str] = []
        with payload.open("rb") as handle:
            for chunk_index in range(DATA_BYTES // CHUNK_BYTES):
                chunk = handle.read(CHUNK_BYTES)
                self.assertEqual(len(chunk), CHUNK_BYTES)
                digest.update(chunk)
                chunk_digests.append(hashlib.sha256(chunk).hexdigest())
                for block_in_chunk in range(CHUNK_BYTES // BLOCK_BYTES):
                    block_index = chunk_index * (CHUNK_BYTES // BLOCK_BYTES) + block_in_chunk
                    begin = block_in_chunk * BLOCK_BYTES
                    self.assertEqual(chunk[begin : begin + 16], expected_block(block_index)[:16])
            self.assertEqual(handle.read(1), b"")
        self.assertEqual(digest.hexdigest(), EXPECTED_SHA256)
        self.assertEqual(len(set(chunk_digests)), 16)

    def test_single_byte_corruption_fails_read(self) -> None:
        payload = self.root / "corrupt.bin"
        parse_success(self.run_probe("write", str(payload)))
        with payload.open("r+b") as handle:
            handle.seek(BLOCK_BYTES + 128)
            handle.write(b"Z")
            handle.flush()
            os.fsync(handle.fileno())
        failed = self.run_probe("read", str(payload))
        self.assertNotEqual(failed.returncode, 0)
        self.assertEqual(failed.stdout, "")

    def test_appended_eof_byte_fails_read(self) -> None:
        payload = self.root / "appended.bin"
        parse_success(self.run_probe("write", str(payload)))
        with payload.open("ab") as handle:
            handle.write(b"x")
            handle.flush()
            os.fsync(handle.fileno())
        failed = self.run_probe("read", str(payload))
        self.assertNotEqual(failed.returncode, 0)
        self.assertEqual(failed.stdout, "")

    def test_write_existing_fails_without_overwrite(self) -> None:
        payload = self.root / "existing.bin"
        payload.write_bytes(b"sentinel")
        failed = self.run_probe("write", str(payload))
        self.assertNotEqual(failed.returncode, 0)
        self.assertEqual(failed.stdout, "")
        self.assertEqual(payload.read_bytes(), b"sentinel")

    def test_invalid_cli_fails(self) -> None:
        cases = [
            (),
            ("write", "relative/path"),
            ("bad-op", str(self.root / "target")),
        ]
        for args in cases:
            with self.subTest(args=args):
                failed = self.run_probe(*args)
                self.assertNotEqual(failed.returncode, 0)
                self.assertEqual(failed.stdout, "")


if __name__ == "__main__":
    unittest.main()
