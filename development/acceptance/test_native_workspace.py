"""Evidence guards for real native workspace admission."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import platform
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('native', Path(__file__).with_name('native-workspace-linux.py'))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class EvidenceGuards(unittest.TestCase):
    def load_mixed(self):
        probe_spec = importlib.util.spec_from_file_location('mixed', Path(__file__).parent / 'probes/native_mixed.py')
        mixed = importlib.util.module_from_spec(probe_spec)
        probe_spec.loader.exec_module(mixed)
        return mixed

    def test_active_node_stop_rejects_idle_before_signalling(self):
        run = object.__new__(native.Run)
        run.root = Path('/opt/guard-only')
        calls = []
        run.native = lambda *args: {'state': 'Idle'}
        run.ctl = lambda *args: calls.append(args)
        run.checks = {}
        with self.assertRaises(ValueError):
            run.stop_active_node(123)
        self.assertEqual(calls, [])

    def test_active_node_stop_signals_node_without_public_workspace_stop(self):
        run = object.__new__(native.Run)
        run.root = Path('/opt/guard-only')
        calls = []
        run.native = lambda *args: calls.append(args) or {'state': 'FinalVerified'}
        run.ctl = lambda *args: calls.append(args)
        run.checks = {}
        with patch.object(Path, 'exists', side_effect=[True, False, False, False]):
            run.stop_active_node(123)
        self.assertEqual(calls, [('active-before-node-stop', 'status'), ('stop', 'node')])
        self.assertTrue(all(row['status'] == 'PASS' for row in run.checks.values()))

    def test_control_artifacts_do_not_multiply_permission_denials(self):
        mixed = self.load_mixed()
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

    def test_append_offset_status_preserves_wrong_offset_failure(self):
        mixed = self.load_mixed()
        self.assertEqual(mixed.append_offset_status([])['status'], 'PARTIAL')
        self.assertEqual(mixed.append_offset_status([{'label': 'primary-1', 'offset': 12, 'expected_eof': 12}])['status'], 'PARTIAL')
        status = mixed.append_offset_status([
            {'label': 'primary-1', 'offset': 12, 'expected_eof': 12},
            {'label': 'secondary-2', 'offset': 24, 'expected_eof': 38},
        ])
        self.assertEqual(status['status'], 'FAIL')
        self.assertEqual(status['failures'][0]['offset'], 24)
        self.assertEqual(status['failures'][0]['expected_eof'], 38)

    def test_concurrent_append_status_rejects_duplicates_and_missing_records(self):
        mixed = self.load_mixed()
        expected = sorted(mixed.expected_append_lines())
        duplicate = expected[:-1] + [expected[0]]
        status = mixed.concurrent_append_status(duplicate, duplicate)
        self.assertEqual(status['status'], 'FAIL')
        self.assertTrue(status['missing'])
        self.assertTrue(status['duplicates'])

    def test_concurrent_offset_correlation_checks_actual_record_end(self):
        mixed = self.load_mixed()
        lines = []
        position = 0
        offsets = {'primary': [], 'secondary': []}
        for i in range(64):
            for role in ('primary', 'secondary'):
                line = f'{role}:{i:02d}:native-mixed-append'
                lines.append(line)
                position += len((line + '\n').encode())
                offsets[role].append({'line': line, 'offset': position})
        content = ('\n'.join(lines) + '\n').encode()
        child_records = [
            {'index': 0, 'returncode': 0, 'timed_out': False, 'stdout': '{"role":"primary","offsets":' + json.dumps(offsets['primary']) + '}'},
            {'index': 1, 'returncode': 0, 'timed_out': False, 'stdout': '{"role":"secondary","offsets":' + json.dumps(offsets['secondary']) + '}'},
        ]
        status = mixed.correlate_concurrent_offsets(content, child_records)
        self.assertEqual(status['status'], 'PASS')
        offsets['primary'][0] = dict(offsets['primary'][0], offset=offsets['primary'][0]['offset'] - 7)
        child_records[0]['stdout'] = '{"role":"primary","offsets":' + json.dumps(offsets['primary']) + '}'
        status = mixed.correlate_concurrent_offsets(content, child_records)
        self.assertEqual(status['status'], 'FAIL')
        self.assertTrue(any(row.get('line') == 'primary:00:native-mixed-append' for row in status['mismatches']))

    def test_concurrent_offset_correlation_rejects_missing_rows_and_duplicates(self):
        mixed = self.load_mixed()
        line = 'primary:00:native-mixed-append'
        content = (line + '\n').encode()
        child_records = [
            {'index': 0, 'returncode': 0, 'timed_out': False,
             'stdout': '{"role":"primary","offsets":[{"line":"primary:00:native-mixed-append","offset":31},{"line":"primary:00:native-mixed-append","offset":31}]}'},
            {'index': 1, 'returncode': 0, 'timed_out': False,
             'stdout': '{"role":"secondary","offsets":[]}'},
        ]
        status = mixed.correlate_concurrent_offsets(content, child_records)
        self.assertEqual(status['status'], 'FAIL')
        self.assertTrue(any(row.get('error') == 'offset coverage mismatch' for row in status['mismatches']))


if __name__ == '__main__':
    unittest.main()
