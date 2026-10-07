"""Linux unit guards for current candidate/dataset identity, not product PASS."""
import copy
import unittest

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


if __name__ == '__main__':
    unittest.main()
