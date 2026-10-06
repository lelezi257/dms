"""Evidence guards for real native workspace admission."""
import hashlib
import importlib.util
import os
from pathlib import Path
import platform
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('native', Path(__file__).with_name('native-workspace-linux.py'))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class EvidenceGuards(unittest.TestCase):
    def test_control_artifacts_do_not_multiply_permission_denials(self):
        probe_spec = importlib.util.spec_from_file_location('mixed', Path(__file__).parent / 'probes/native_mixed.py')
        mixed = importlib.util.module_from_spec(probe_spec)
        probe_spec.loader.exec_module(mixed)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for suffix, text in [('command.json', '{}'), ('exit.json', '{}'),
                                 ('stdout', 'uid=501'), ('stderr', 'Permission denied')]:
                (root / ('command-0001.' + suffix)).write_text(text)
            records = mixed.find_command_artifacts(root, set())
            self.assertEqual(len(records), 1)
            self.assertEqual(sum(r.get('stderr', {}).get('text', '').count('Permission denied')
                                 for r in records), 1)

    @unittest.skipUnless(platform.system() == 'Linux', 'procfs descriptor paths require Linux')
    def test_lock_probe_preserves_descriptor_path(self):
        probe_spec = importlib.util.spec_from_file_location('locks', Path(__file__).parent / 'probes/locks_smoke.py')
        locks = importlib.util.module_from_spec(probe_spec)
        probe_spec.loader.exec_module(locks)
        with tempfile.TemporaryDirectory() as tmp:
            fd = os.open(tmp, os.O_RDONLY | os.O_DIRECTORY)
            try:
                path = Path(f'/proc/self/fd/{fd}/lock-target')
                args = locks.build_parser().parse_args(['--path', str(path), '--second-path', str(path),
                    '--evidence', str(Path(tmp) / 'evidence')])
                probe = locks.Probe(args)
                self.assertEqual(probe.primary_path, path)
                self.assertEqual(probe.secondary_path, path)
                probe.setup()
                self.assertEqual(path.read_bytes(), (Path(tmp) / 'lock-target').read_bytes())
            finally:
                os.close(fd)

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
