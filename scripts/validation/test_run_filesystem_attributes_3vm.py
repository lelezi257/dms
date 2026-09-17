from __future__ import annotations

import argparse
import unittest

import run_filesystem_attributes_3vm as runner


class AttributesThreeVmRunnerTests(unittest.TestCase):
    def test_run_id_rejects_remote_path_injection(self) -> None:
        self.assertEqual(runner.validated_run_id("attributes_01"), "attributes_01")
        for invalid in ("../escape", "has/slash", "$(command)", "space value", ""):
            with self.assertRaises(argparse.ArgumentTypeError):
                runner.validated_run_id(invalid)


if __name__ == "__main__":
    unittest.main()
