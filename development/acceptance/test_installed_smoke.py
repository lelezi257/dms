"""Negative evidence checks for the installed regression; run only on Linux."""
import copy
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('installed_smoke', Path(__file__).with_name('installed-smoke-linux.py'))
tool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tool)


class RecoveryEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.before = {'meta': {'pid': 1, 'starttick': 100, 'sha256': 'meta'},
                       'node': {'pid': 2, 'starttick': 110, 'sha256': 'node'},
                       'mounts': {'ownerfs': {'target': '/owner'}, 'dfs': {'target': '/dfs'}}}
        self.after = copy.deepcopy(self.before)
        self.after['meta'].update(pid=3, starttick=200)
        self.writes = {name: {'status': 'PASS', 'size': 64 * 2**20, 'sha256': name}
                       for name in ('ownerfs', 'dfs')}
        self.reads = copy.deepcopy(self.writes)

    def verify(self):
        tool.verify_recovery(self.before, self.after, self.writes, self.reads)

    def test_unchanged_node_mounts_new_meta_and_contents(self):
        self.verify()

    def test_refuses_node_restart(self):
        self.after['node']['starttick'] += 1
        with self.assertRaises(ValueError):
            self.verify()

    def test_refuses_mount_change(self):
        self.after['mounts']['dfs']['target'] = '/another'
        with self.assertRaises(ValueError):
            self.verify()

    def test_refuses_no_restart_wrong_binary_or_start_order(self):
        for field, value in (('pid', 1), ('sha256', 'other'), ('starttick', 100)):
            with self.subTest(field=field):
                original = self.after['meta'][field]
                self.after['meta'][field] = value
                with self.assertRaises(ValueError):
                    self.verify()
                self.after['meta'][field] = original

    def test_refuses_either_backend_corruption_short_read_or_failed_probe(self):
        for name in ('ownerfs', 'dfs'):
            for field, value in (('sha256', 'corrupt'), ('size', 1024), ('status', 'FAIL')):
                with self.subTest(name=name, field=field):
                    original = self.reads[name][field]
                    self.reads[name][field] = value
                    with self.assertRaises(ValueError):
                        self.verify()
                    self.reads[name][field] = original


class InstalledExecutableTests(unittest.TestCase):
    def test_accepts_symlink_to_installed_file(self):
        with tempfile.TemporaryDirectory() as directory:
            installed = Path(directory) / 'installed'
            installed.write_bytes(b'same ELF')
            running = Path(directory) / 'running'
            running.symlink_to(installed)
            self.assertEqual(tool.verify_executable(running, installed)['inode'],
                             installed.stat().st_ino)

    def test_refuses_same_bytes_in_another_file_or_hardlink_path(self):
        with tempfile.TemporaryDirectory() as directory:
            installed = Path(directory) / 'installed'
            installed.write_bytes(b'same ELF')
            another = Path(directory) / 'other'
            another.write_bytes(installed.read_bytes())
            link = Path(directory) / 'link'
            os.link(installed, link)
            for candidate in (another, link):
                with self.subTest(candidate=candidate):
                    with self.assertRaises(ValueError):
                        tool.verify_executable(candidate, installed)


if __name__ == '__main__':
    unittest.main()
