import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('archive', Path(__file__).with_name('archive-extractions-v65.py'))
archive = importlib.util.module_from_spec(spec)
spec.loader.exec_module(archive)


class ArchiveTests(unittest.TestCase):
    def fixture(self, base):
        source = base / 'source'
        source.mkdir()
        (source/'binary').write_bytes(b'preserved executable bytes\0')
        os.chmod(source/'binary', 0o755)
        os.setxattr(source/'binary', 'user.archive-test', b'preserved attribute')
        os.symlink('binary', source/'alias')
        return source, base/'destination'

    def test_copy_link_and_manual_restore_preserve_all_recorded_attributes(self):
        with tempfile.TemporaryDirectory() as d:
            source, destination = self.fixture(Path(d))
            before = archive.manifest(source)
            record = archive.archive_one(source, destination)
            self.assertEqual(record['entries'], before)
            self.assertTrue(source.is_symlink())
            self.assertEqual(archive.manifest(destination), before)
            self.assertEqual((source/'alias').read_bytes(), (destination/'binary').read_bytes())
            restore = source.with_name('restoring')
            archive.subprocess.run(['cp','-a','--',str(destination),str(restore)], check=True)
            self.assertEqual(archive.manifest(restore), before)
            source.unlink()
            os.rename(restore, source)
            self.assertEqual(archive.manifest(source), before)

    def test_live_fd_blocks_before_copy_or_rename(self):
        with tempfile.TemporaryDirectory() as d:
            source, destination = self.fixture(Path(d))
            with (source/'binary').open('rb'):
                with self.assertRaisesRegex(RuntimeError, 'active_paths'):
                    archive.archive_one(source, destination)
            self.assertFalse(destination.exists())
            self.assertFalse(source.is_symlink())

    def test_existing_destination_is_never_overwritten(self):
        with tempfile.TemporaryDirectory() as d:
            source, destination = self.fixture(Path(d))
            destination.mkdir()
            with self.assertRaisesRegex(RuntimeError, 'already exists'):
                archive.archive_one(source, destination)
            self.assertFalse(source.is_symlink())

    def test_special_and_hardlinked_files_are_rejected(self):
        with tempfile.TemporaryDirectory() as d:
            source, destination = self.fixture(Path(d))
            os.link(source/'binary', source/'hardlink')
            with self.assertRaisesRegex(RuntimeError, 'hardlinked'):
                archive.manifest(source)
            (source/'hardlink').unlink()
            os.mkfifo(source/'fifo')
            with self.assertRaisesRegex(RuntimeError, 'special'):
                archive.manifest(source)

    def test_corrupted_copy_keeps_original_path_unchanged(self):
        with tempfile.TemporaryDirectory() as d:
            source, destination = self.fixture(Path(d))
            before = archive.manifest(source)
            real_run = archive.subprocess.run
            def corrupt(*args, **kwargs):
                result = real_run(*args, **kwargs)
                (destination/'binary').write_bytes(b'corrupted copy')
                return result
            with patch.object(archive.subprocess, 'run', side_effect=corrupt):
                with self.assertRaisesRegex(RuntimeError, 'copy mismatch'):
                    archive.archive_one(source, destination)
            self.assertFalse(source.is_symlink())
            self.assertEqual(archive.manifest(source), before)
            self.assertTrue(destination.exists())

    def test_link_install_failure_restores_original_directory(self):
        with tempfile.TemporaryDirectory() as d:
            source, destination = self.fixture(Path(d))
            before = archive.manifest(source)
            with patch.object(archive.os, 'symlink', side_effect=OSError('injected link failure')):
                with self.assertRaisesRegex(OSError, 'injected link failure'):
                    archive.archive_one(source, destination)
            self.assertFalse(source.is_symlink())
            self.assertEqual(archive.manifest(source), before)
            self.assertEqual(archive.manifest(destination), before)

    def test_mountinfo_detects_foreign_namespace_and_escaped_paths(self):
        info = '30 20 0:4 / /tmp/source/private rw - ext4 /dev/test rw\n'
        self.assertEqual(archive.mount_conflicts(info, [Path('/tmp/source')]), ['/tmp/source/private'])
        escaped = '30 20 0:4 / /tmp/a\\040b/private rw - ext4 /dev/test rw\n'
        self.assertEqual(archive.mount_conflicts(escaped, [Path('/tmp/a b')]), ['/tmp/a b/private'])
        self.assertEqual(archive.mount_conflicts(info, [Path('/tmp/sour')]), [])

    def test_preflight_rejects_insufficient_reserve_and_same_device(self):
        gib=1024**3
        archive.preflight_space(gib, 10*gib, 4*gib, 4*gib, False)
        for args, message in [((gib,10*gib,2*gib,2*gib,False),'cannot meet'),
                              ((gib,6*gib,4*gib,4*gib,False),'insufficient archive'),
                              ((gib,10*gib,4*gib,4*gib,True),'would not release')]:
            with self.subTest(args=args), self.assertRaisesRegex(RuntimeError,message):
                archive.preflight_space(*args)

    def test_manifest_detects_same_size_content_and_metadata_changes(self):
        with tempfile.TemporaryDirectory() as d:
            source, destination = self.fixture(Path(d))
            before = archive.manifest(source)
            f = source/'binary'
            original = f.read_bytes()
            timestamp = f.stat().st_mtime_ns
            f.write_bytes(b'X'+original[1:])
            os.utime(f, ns=(timestamp,timestamp))
            self.assertNotEqual(archive.manifest(source), before)
            f.write_bytes(original)
            os.utime(f, ns=(timestamp,timestamp))
            self.assertEqual(archive.manifest(source), before)
            os.chmod(f, 0o700)
            self.assertNotEqual(archive.manifest(source), before)


if __name__ == '__main__':
    unittest.main(verbosity=2)
