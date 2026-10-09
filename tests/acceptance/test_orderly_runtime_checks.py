"""Negative controls for package qualification and directly observed orderly closure."""
import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import platform
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('outer_checks', Path(__file__).with_name('orderly-runtime-checks.py'))
checks = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checks)
spec = importlib.util.spec_from_file_location('orderly_probe', Path(__file__).parent / 'probes/native_orderly_recovery.py')
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)


@unittest.skipUnless(platform.system() == 'Linux', 'verification is Linux only')
class Guards(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)

    def tearDown(self):
        self.temporary.cleanup()

    def package(self):
        blob = b'\x7fELF\x02\x01' + bytes(12) + b'\xb7\x00' + bytes(40)
        path = self.root / 'package.tar.gz'
        manifest = {'source_commit': '3' * 40}
        with tarfile.open(path, 'w:gz') as archive:
            for name, data in [('p/manifest.json', json.dumps(manifest).encode()),
                               ('p/bin/afs-meta', blob), ('p/bin/afs-node', blob)]:
                member = tarfile.TarInfo(name)
                member.size = len(data)
                archive.addfile(member, io.BytesIO(data))
        expected = {'source_commit': '3' * 40, 'package_sha256': checks.sha(path),
                    'afs_meta_sha256': hashlib.sha256(blob).hexdigest(),
                    'afs_node_sha256': hashlib.sha256(blob).hexdigest()}
        return path, expected

    def test_exact_new_package_then_wrong_hash_source_and_elf_rejected(self):
        path, expected = self.package()
        checks.package_inputs(path, expected)
        for key in expected:
            altered = {**expected, key: '0' * len(expected[key])}
            with self.subTest(key=key), self.assertRaises(ValueError):
                checks.package_inputs(path, altered)

    def receipts(self):
        oracle = {'phase1': {'process': {}, 'actual_waits': {}}, 'phase2': {'process': {}}}
        final = {}
        for number, (phase, saved) in enumerate((('phase1', oracle['phase1']['actual_waits']), ('phase2', final))):
            for offset, role in enumerate(('meta', 'node')):
                pid = 100 + number * 4 + offset * 2
                path = self.root / 'run' / (role + '.lifecycle.' + str(number))
                actual_path = self.root / 'phase1/run' / path.name if number == 0 else path
                actual_path.mkdir(parents=True)
                executable = self.root / 'prefix/bin' / ('afs-' + role)
                service = {'pid': pid, 'starttick': pid * 10, 'boot_id': 'boot',
                           'installed': {'path': str(executable)}}
                child = {'pid': str(pid), 'supervisor_pid': str(pid + 1), 'exe': str(executable),
                         'config': str(self.root / 'etc' / (role + '.toml')),
                         'start_ticks': str(pid * 10), 'boot_id': 'boot', 'lifecycle': str(path)}
                receipt, ready = {**child, 'exit_code': '0'}, {'supervisor_pid': str(pid + 1)}
                for name, value in (('child', child), ('exit', receipt), ('ready', ready)):
                    (actual_path / name).write_text(''.join(k + '=' + v + '\n' for k, v in value.items()))
                oracle[phase]['process'][role] = service
                saved[role] = {'path': str(path), 'child': child, 'exit': receipt, 'ready': ready, 'both_gone': True}
        return oracle, final

    def test_four_actual_wait0_eight_distinct_pids(self):
        oracle, final = self.receipts()
        result = checks.verify_waits(self.root, oracle, final, probe, self.root / 'phase1/run', exists=lambda pid: False)
        self.assertEqual(len(result['receipts']), 4)
        self.assertEqual(len(result['eight_pids_gone']), 8)

    def test_missing_actual_exit_not_replaced_by_saved_pass(self):
        oracle, final = self.receipts()
        (Path(final['node']['path']) / 'exit').unlink()
        with self.assertRaises(FileNotFoundError):
            checks.verify_waits(self.root, oracle, final, probe, self.root / 'phase1/run', exists=lambda pid: False)

    def test_wrong_incarnation_rejected_even_when_saved_receipt_matches(self):
        oracle, final = self.receipts()
        oracle['phase2']['process']['node']['starttick'] += 1
        with self.assertRaises(ValueError):
            checks.verify_waits(self.root, oracle, final, probe, self.root / 'phase1/run', exists=lambda pid: False)

    def test_nonzero_wait_rejected_even_when_saved_receipt_matches(self):
        oracle, final = self.receipts()
        item = final['node']
        item['exit']['exit_code'] = '1'
        path = Path(item['path']) / 'exit'
        path.write_text(path.read_text().replace('exit_code=0', 'exit_code=1'))
        with self.assertRaises(ValueError):
            checks.verify_waits(self.root, oracle, final, probe, self.root / 'phase1/run', exists=lambda pid: False)

    def test_actual_child_or_supervisor_residue_rejected(self):
        oracle, final = self.receipts()
        for pid in (106, 107):
            with self.subTest(pid=pid), self.assertRaises(ValueError):
                checks.verify_waits(self.root, oracle, final, probe, self.root / 'phase1/run', exists=lambda actual: actual == pid)

    def test_foreign_and_extra_lifecycle_rejected(self):
        oracle, final = self.receipts()
        (self.root / 'run/meta.lifecycle.foreign').mkdir()
        with self.assertRaises(ValueError):
            checks.verify_waits(self.root, oracle, final, probe, self.root / 'phase1/run', exists=lambda pid: False)

    def test_missing_phase1_archived_receipt_rejected_without_live_fallback(self):
        oracle, final = self.receipts()
        path = Path(oracle['phase1']['actual_waits']['meta']['path'])
        archived = self.root / 'phase1/run' / path.name
        path.mkdir()
        for name in ('child', 'exit', 'ready'):
            (path / name).write_bytes((archived / name).read_bytes())
        (archived / 'exit').unlink()
        with self.assertRaises(FileNotFoundError):
            checks.verify_waits(self.root, oracle, final, probe, self.root / 'phase1/run', exists=lambda pid: False)

    def test_budget_and_floor_boundary_no_lowered_predicates(self):
        checks.verify_budget(checks.CEILING, checks.FLOOR)
        checks.verify_budget(0, checks.CEILING + checks.FLOOR, initial=True)
        for size, free, initial in ((checks.CEILING + 1, checks.FLOOR, False),
                                    (0, checks.FLOOR - 1, False),
                                    (0, checks.FLOOR, True), (True, checks.FLOOR, False)):
            with self.subTest(size=size, free=free, initial=initial), self.assertRaises(ValueError):
                checks.verify_budget(size, free, initial)

    def test_ram_available_and_finite_ancestor_headroom(self):
        limits = [{'maximum': 'max', 'current': 100},
                  {'maximum': 2 * checks.RAM_MARGIN, 'current': checks.RAM_MARGIN}]
        checks.verify_ram(checks.RAM_MARGIN, limits)
        with self.assertRaises(ValueError):
            checks.verify_ram(checks.RAM_MARGIN - 1, limits)
        limits[-1]['current'] += 1
        with self.assertRaises(ValueError):
            checks.verify_ram(8 * checks.RAM_MARGIN, limits)

    def test_protected_incarnation_and_exact_mount_row_rejected(self):
        before = {'boot_id': 'boot', 'processes': [{'pid': 7, 'starttick': 8}],
                  'mountinfo': ['41 40 0:1 / /protected rw - ext4 /dev/vda rw']}
        checks.verify_protected(before, copy.deepcopy(before))
        for key in ('boot_id', 'processes', 'mountinfo'):
            after = copy.deepcopy(before)
            after[key] = 'wrong' if key == 'boot_id' else []
            with self.subTest(key=key), self.assertRaises(ValueError):
                checks.verify_protected(before, after)

    def test_wrong_mode_or_product_identity_rejected(self):
        expected = {'source_commit': '3' * 40}
        result = {'status': 'PASS', 'cleanup': 'PASS', 'source_commit': '3' * 40,
                  'driver_sha256': 'a' * 64, 'orderly_recovery_selected': True,
                  'basic_payload_selected': False, 'source_rejection_selected': False,
                  'control_capacity_selected': False, 'semantic_groups': []}
        checks.verify_driver(result, expected, 'a' * 64)
        for key, value in (('source_commit', '6' * 40), ('driver_sha256', 'b' * 64),
                           ('basic_payload_selected', True), ('orderly_recovery_selected', False)):
            with self.subTest(key=key), self.assertRaises(ValueError):
                checks.verify_driver({**result, key: value}, expected, 'a' * 64)

    def test_atomic_complete_receipt_never_overwrites_original(self):
        path = self.root / 'report.json'
        checks.publish(path, {'all': list(range(300))})
        self.assertEqual(checks.read_json(path), {'all': list(range(300))})
        original = path.read_bytes()
        with self.assertRaises(FileExistsError):
            checks.publish(path, {'replaced': True})
        self.assertEqual(path.read_bytes(), original)
        self.assertEqual(list(self.root.iterdir()), [path])


if __name__ == '__main__':
    unittest.main()
