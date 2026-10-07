"""Guard the fixed paired-performance criterion and accounting."""
import copy
import importlib.util
from pathlib import Path
import unittest


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(file))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


runner = load('workspace_data_perf', 'workspace-bind-data-perf-linux.py')
fixtures = load('old_perf_guards', 'test_container_perf.py')


def rounds():
    result = []
    for index in range(6):
        order = ['experiment', 'reference'] if index % 2 == 0 else ['reference', 'experiment']
        samples = []
        for name in order:
            samples.append(dict(target=name, write=fixtures.io_result('seq-write'),
                                read=fixtures.io_result('seq-read')))
        result.append(dict(round=index, measured=index > 0, order=order, samples=samples))
    return result


class PairedDataGuards(unittest.TestCase):
    def test_direction_threshold_and_warmup_are_fixed(self):
        observed = rounds()
        for round_value in observed:
            by_name = {s['target']: s for s in round_value['samples']}
            by_name['experiment']['write']['wall_ns'] = 2000
            by_name['reference']['write']['wall_ns'] = 1000
        observed[0]['samples'][0]['read']['wall_ns'] = 10**9
        result = runner.paired_analysis(observed)
        self.assertEqual(result['write']['median'], 0.5)
        self.assertEqual(result['write']['status'], 'FAIL')
        self.assertEqual(result['read']['median'], 1)
        self.assertEqual(result['read']['status'], 'PASS')
        self.assertEqual(len(result['read']['paired_speed_ratios']), 5)

    def test_reject_missing_pair_round_duplicate_and_changed_order(self):
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

    def test_reject_content_and_cache_claims(self):
        for field, invalid in [('content_ok', False), ('cache_requested', 'hot'), ('wall_ns', 0)]:
            value = copy.deepcopy(rounds())
            value[2]['samples'][0]['read'][field] = invalid
            with self.subTest(field=field), self.assertRaises(ValueError):
                runner.paired_analysis(value)


if __name__ == '__main__':
    unittest.main()
