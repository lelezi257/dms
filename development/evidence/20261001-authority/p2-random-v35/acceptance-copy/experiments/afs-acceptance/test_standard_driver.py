import importlib.util
import sys
import unittest
from pathlib import Path

drivers = Path(__file__).parent / "drivers"
sys.path.insert(0, str(drivers))
spec = importlib.util.spec_from_file_location("standard_driver", drivers / "standard.py")
standard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(standard)


class StandardAccountingTest(unittest.TestCase):
    def setUp(self):
        self.counts = {"tap_ok": 7, "tap_not_ok": 1, "prove_files": 2, "prove_tests": 8, "tap_planned": 8}

    def test_complete(self):
        self.assertTrue(standard.complete_accounting(self.counts, 2))

    def test_lost_file(self):
        self.assertFalse(standard.complete_accounting(self.counts, 3))

    def test_lost_subtest(self):
        self.counts["tap_planned"] = 9
        self.assertFalse(standard.complete_accounting(self.counts, 2))

    def test_missing_summary(self):
        self.counts["prove_tests"] = None
        self.assertFalse(standard.complete_accounting(self.counts, 2))

    def test_empty_pass(self):
        self.counts.update(tap_ok=0, tap_not_ok=0, prove_tests=0, tap_planned=0)
        self.assertFalse(standard.complete_accounting(self.counts, 2))


if __name__ == "__main__":
    unittest.main()
