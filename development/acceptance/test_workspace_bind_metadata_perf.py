"""Reject invalid measurement evidence and keep six phase decisions independent."""
import copy
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('metadata_perf', Path(__file__).with_name('workspace-bind-metadata-perf-linux.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


def payload():
    phases = []
    for name in runner.PHASES:
        value = dict(name=name, wall_ns=1000, client_cpu_ns=100, operations=1 if name == 'readdir' else 1000)
        if name != 'readdir':
            value.update(p50_ns=10, p95_ns=20, p99_ns=30)
        phases.append(value)
    return dict(files=1000, file_bytes=4096, concurrency=1, path_form='absolute',
                barrier='close visibility; durability unqualified', phases=phases)


def rounds():
    values = []
    for index in range(6):
        order = ['experiment', 'reference'] if index % 2 == 0 else ['reference', 'experiment']
        values.append(dict(round=index, measured=index > 0, order=order,
                           samples=[dict(target=name, metadata=payload()) for name in order]))
    return values


class MetadataGuards(unittest.TestCase):
    def test_six_phase_decisions_warmup_and_ratio_direction(self):
        value = rounds()
        for item in value:
            sample = next(s for s in item['samples'] if s['target'] == 'experiment')
            sample['metadata']['phases'][0]['wall_ns'] = 2000
        value[0]['samples'][0]['metadata']['phases'][1]['wall_ns'] = 10**9
        result = runner.paired_analysis(value)
        self.assertEqual(set(result), set(runner.PHASES))
        self.assertEqual(result['create_write_close']['median'], .5)
        self.assertEqual(result['create_write_close']['status'], 'FAIL')
        self.assertEqual(result['stat']['median'], 1)
        self.assertEqual(result['stat']['status'], 'PASS')
        self.assertEqual(len(result['stat']['paired_speed_ratios']), 5)

    def test_invalid_pairs_do_not_produce_results(self):
        for change in ('missing', 'duplicate', 'order', 'warmup'):
            value = rounds()
            if change == 'missing':
                value.pop()
            elif change == 'duplicate':
                value[1]['samples'][0]['target'] = value[1]['samples'][1]['target']
            elif change == 'order':
                value[1]['order'].reverse()
            else:
                value[0]['measured'] = True
            with self.subTest(change=change), self.assertRaises(ValueError):
                runner.paired_analysis(value)

    def test_changed_shape_and_barrier_are_rejected(self):
        for key, changed in [('files', 999), ('file_bytes', 8192), ('concurrency', 8),
                             ('path_form', 'relative'), ('barrier', 'durable')]:
            value = payload()
            value[key] = changed
            with self.subTest(key=key), self.assertRaises(ValueError):
                runner.verify_payload(value)

    def test_operation_latency_and_timing_accounting(self):
        for key, changed in [('operations', 999), ('operations', True), ('client_cpu_ns', -1),
                             ('wall_ns', True), ('p50_ns', 0), ('p95_ns', 31)]:
            value = payload()
            value['phases'][0][key] = changed
            with self.subTest(key=key, changed=changed), self.assertRaises(ValueError):
                runner.verify_payload(value)
        value = payload()
        value['phases'][3]['operations'] = 1000
        with self.assertRaises(ValueError):
            runner.verify_payload(value)

    def test_missing_or_reordered_phase_is_rejected(self):
        for change in ('missing', 'reorder'):
            value = copy.deepcopy(payload())
            if change == 'missing':
                value['phases'].pop()
            else:
                value['phases'].reverse()
            with self.subTest(change=change), self.assertRaises(ValueError):
                runner.verify_payload(value)


if __name__ == '__main__':
    unittest.main()
