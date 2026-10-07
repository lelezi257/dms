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


class ActualWaitEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.child = {'pid': '41', 'start_ticks': '100', 'boot_id': 'boot',
                      'exe': '/owned/prefix/bin/afs-meta', 'config': '/owned/etc/meta.toml',
                      'lifecycle': '/owned/run/meta.lifecycle.first', 'supervisor_pid': '40'}
        self.ready = {'supervisor_pid': '40'}
        self.captured = {'pid': 41, 'starttick': 100, 'boot_id': 'boot',
                         'executable': {'path': '/owned/prefix/bin/afs-meta'},
                         'lifecycle': {'path': self.child['lifecycle'], 'child': copy.deepcopy(self.child),
                                       'ready': copy.deepcopy(self.ready), 'supervisor': {'pid': 40}}}
        self.receipt = dict(self.child, exit_code='0')

    def verify(self, gone=True):
        tool.verify_wait(self.captured, self.child, self.ready, self.receipt, gone)

    def test_accepts_exact_bound_actual_wait_and_both_gone(self):
        self.verify()

    def test_refuses_foreign_wait_or_nonzero_exit(self):
        for key, value in (('lifecycle', '/foreign/run/meta.lifecycle.other'),
                           ('supervisor_pid', '39'), ('exit_code', '1')):
            with self.subTest(key=key):
                original = self.receipt[key]
                self.receipt[key] = value
                with self.assertRaises(ValueError):
                    self.verify()
                self.receipt[key] = original

    def test_refuses_changed_captured_pid_tick_or_boot(self):
        for key, value in (('pid', 42), ('starttick', 101), ('boot_id', 'another-boot')):
            with self.subTest(key=key):
                original = self.captured[key]
                self.captured[key] = value
                with self.assertRaises(ValueError):
                    self.verify()
                self.captured[key] = original

    def test_refuses_changed_live_child_or_supervisor_ready(self):
        self.child['start_ticks'] = '101'
        self.receipt['start_ticks'] = '101'
        with self.assertRaises(ValueError):
            self.verify()
        self.child['start_ticks'] = self.receipt['start_ticks'] = '100'
        self.ready['supervisor_pid'] = '39'
        with self.assertRaises(ValueError):
            self.verify()

    def test_refuses_missing_actual_process_absence(self):
        for value in (False, None, 1):
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    self.verify(value)


class InstalledConfigEvidenceTests(unittest.TestCase):
    def configs(self, name):
        generated = {'fs': 'all', 'dfs_desired_copies': 1, 'dfs_sync_required_copies': 1,
                     'dfs_min_distinct_nodes': 1, 'dfs_min_distinct_failure_domains': 1,
                     'dfs_local_copy': 'required'}
        if name == 'meta':
            generated['meta_store'] = 'local-file'
        else:
            generated.update(data_mode='grpc', allow_volatile_meta=False)
        effective = dict(generated, ownerfs=True, dfs=True, experimental_native_workspace=False,
                         experimental_ownerfs_workspace_bind=False)
        return generated, effective

    def test_accepts_generated_defaults_and_effective_off_r1_both_fs(self):
        for name in ('meta', 'node'):
            tool.verify_config(name, *self.configs(name))

    def test_refuses_either_workspace_switch_or_missing_effective_default(self):
        for name in ('meta', 'node'):
            for key in ('experimental_native_workspace', 'experimental_ownerfs_workspace_bind'):
                for target, value in (('generated', True), ('effective', True), ('effective', None)):
                    with self.subTest(name=name, key=key, target=target, value=value):
                        generated, effective = self.configs(name)
                        (generated if target == 'generated' else effective)[key] = value
                        with self.assertRaises(ValueError):
                            tool.verify_config(name, generated, effective)

    def test_refuses_wrong_backend_fs_or_r1_policy(self):
        for name in ('meta', 'node'):
            bad = [('fs', 'dfs'), ('dfs_desired_copies', 2), ('dfs_sync_required_copies', 2),
                   ('dfs_min_distinct_nodes', 2), ('dfs_min_distinct_failure_domains', True),
                   ('dfs_local_copy', 'disabled')]
            bad += [('meta_store', 'memory')] if name == 'meta' else [('data_mode', 'rdma'), ('allow_volatile_meta', True)]
            for key, value in bad:
                for target in ('generated', 'effective'):
                    if key == 'fs' and target == 'effective':
                        continue
                    with self.subTest(name=name, key=key, target=target):
                        generated, effective = self.configs(name)
                        (generated if target == 'generated' else effective)[key] = value
                        with self.assertRaises(ValueError):
                            tool.verify_config(name, generated, effective)
            generated, effective = self.configs(name)
            effective['ownerfs'] = False
            with self.assertRaises(ValueError):
                tool.verify_config(name, generated, effective)


if __name__ == '__main__':
    unittest.main()
