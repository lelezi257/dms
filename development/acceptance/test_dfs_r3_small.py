"""Linux unit guards for current candidate/dataset identity, not product PASS."""
import copy
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import dfs_r3_small as probe


class CurrentDatasetGuards(unittest.TestCase):
    def setUp(self):
        self.identity = dict(product_source_commit=probe.PRODUCT, compiler_input_map=probe.MAP,
                             afs_meta_sha256=probe.META_SHA, afs_node_sha256=probe.NODE_SHA,
                             io_sha256='0' * 64)

    def test_counter_dataset_cannot_deduplicate_to_one_chunk(self):
        shape = probe.expected_content()
        self.assertEqual(shape['bytes'], 64 * 2**20)
        self.assertEqual(len(set(shape['chunk_sha256'])), 16)
        self.assertNotEqual(shape['sha256'], probe.sync.expected_payload_sha())

    def test_historical_identity_is_not_current(self):
        for key in ('product_source_commit', 'compiler_input_map', 'afs_node_sha256', 'afs_meta_sha256'):
            altered = dict(self.identity, **{key: 'historical'})
            with self.assertRaisesRegex(ValueError, 'mismatch'):
                probe.validate_identity(altered)
        self.assertEqual(probe.validate_identity(self.identity), self.identity)

    def test_manifest_requires_writer_success_exact_shape_and_identity(self):
        valid = dict(status='DATA_RECORDED', identity=self.identity, relative_dir=probe.RELATIVE,
                     content=probe.expected_content())
        self.assertEqual(probe.validate_manifest(valid, self.identity), valid)
        for mutation in ({'status': 'FAIL'}, {'relative_dir': '../escape'}, {'content': {}}, {'identity': {}}):
            with self.assertRaises(ValueError):
                probe.validate_manifest(dict(valid, **mutation), self.identity)

    def test_uniform_or_partial_result_rejected(self):
        row = dict(operation='seq-read', file_bytes=64 * 2**20, io_bytes=64 * 2**20,
                   block_bytes=2**20, concurrency=1, barrier='close', pattern_byte=97,
                   operations=64, cache_requested='unobserved', content_ok=True,
                   residency_observed=False, wall_ns=1, dataset=probe.DATASET)
        probe.validate_sample(row, 'read')
        for mutation in ({'dataset': 'uniform'}, {'operations': 63}, {'wall_ns': True}, {'content_ok': False}):
            with self.assertRaises(ValueError):
                probe.validate_sample(dict(row, **mutation), 'read')

    def test_explicit_candidate_does_not_inherit_old_identity(self):
        candidate = dict(self.identity, product_source_commit='a' * 40,
                         compiler_input_map='b' * 64, afs_meta_sha256='c' * 64,
                         afs_node_sha256='d' * 64, io_sha256='e' * 64)
        self.assertEqual(probe.validate_identity(candidate, candidate), candidate)
        with self.assertRaisesRegex(ValueError, 'mismatch'):
            probe.validate_identity(self.identity, candidate)
        for key in candidate:
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, 'mismatch'):
                probe.validate_identity(dict(candidate, **{key: '0' * len(candidate[key])}), candidate)

    def test_explicit_candidate_missing_or_malformed_cannot_fall_back(self):
        candidate = dict(self.identity, product_source_commit='a' * 40)
        for key in candidate:
            for bad in (None, True, 'G' * len(candidate[key]), candidate[key][:-1]):
                with self.subTest(key=key, bad=bad), self.assertRaisesRegex(ValueError, 'invalid candidate'):
                    probe.validate_identity(candidate, dict(candidate, **{key: bad}))
            absent = dict(candidate)
            del absent[key]
            with self.assertRaisesRegex(ValueError, 'invalid candidate'):
                probe.validate_identity(candidate, absent)

    def test_single_read_needs_probe_and_final_content_success(self):
        manifest = dict(status='DATA_RECORDED', identity=self.identity,
                        relative_dir=probe.RELATIVE, content=probe.expected_content())
        args = SimpleNamespace(manifest='/manifest.json', round_timeout=60)
        verified = dict(status='PASS', sha256=manifest['content']['sha256'])
        for sample, final in (({'status': 'FAIL'}, verified),
                              ({'status': 'PASS'}, ValueError('post-read corruption')),
                              ({'status': 'PASS'}, verified)):
            with tempfile.TemporaryDirectory() as directory:
                out = Path(directory)
                with patch.object(probe, 'prepare', return_value=(out, out, out/'payload', out/'io', self.identity)), \
                     patch.object(probe.sync, 'read_json', return_value=manifest), \
                     patch.object(probe, 'verify_content', side_effect=[verified, final]), \
                     patch.object(probe, 'run_sample', return_value=sample) as run:
                    result = probe.check_reader(args)
                    run.assert_called_once_with(out/'io', out/'payload', 'read', out, 0, 60)
                    self.assertEqual(result['status'], 'DATA_RECORDED' if sample['status'] == 'PASS'
                                     and not isinstance(final, Exception) else 'BLOCKED')
                    if result['status'] == 'DATA_RECORDED':
                        self.assertFalse(result['performance_claim'])
                    self.assertTrue((out/'summary.json').is_file())


if __name__ == '__main__':
    unittest.main()
