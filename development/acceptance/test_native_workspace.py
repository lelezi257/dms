"""Evidence guards for real native workspace admission."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('native', Path(__file__).with_name('native-workspace-linux.py'))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class EvidenceGuards(unittest.TestCase):
    def test_final_claim_rejects_wrong_namespace_source_and_flags(self):
        source, namespace = {'dev': 1, 'ino': 2}, {'dev': 4, 'ino': 5}
        good = {'source': source, 'namespace': namespace, 'unique_mount_id': 9, 'flags': ['nosuid', 'nodev']}
        native.verify_final(good, source, namespace)
        for field, wrong in [('source', {'dev': 1, 'ino': 99}), ('namespace', {'dev': 4, 'ino': 99}),
                             ('unique_mount_id', 0), ('unique_mount_id', None), ('flags', ['nosuid'])]:
            with self.subTest(field=field, wrong=wrong), self.assertRaises(ValueError):
                native.verify_final(dict(good, **{field: wrong}), source, namespace)

    def test_size_and_content_required(self):
        with tempfile.TemporaryDirectory() as tmp:
            p = Path(tmp) / 'payload'
            p.write_bytes(b'good')
            digest = hashlib.sha256(b'good').hexdigest()
            self.assertTrue(native.content_matches(p, 4, digest))
            self.assertFalse(native.content_matches(p, 5, digest))
            p.write_bytes(b'evil')
            self.assertFalse(native.content_matches(p, 4, digest))


if __name__ == '__main__':
    unittest.main()
