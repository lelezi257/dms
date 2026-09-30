import importlib.util
import json
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("target_identity", Path(__file__).parent / "drivers/target_identity.py")
target = importlib.util.module_from_spec(spec)
spec.loader.exec_module(target)


def mount(source="afs-dfs", filesystem="fuse", target="/mount"):
    return {"returncode": 0, "stdout": json.dumps({"filesystems": [{"source": source, "fstype": filesystem, "target": target}]})}


def process(name):
    return {"exists": True, "exe": "/bin/" + name, "exe_sha256": "a" * 64, "cmdline": name + " --config /config"}


class TargetIdentityTest(unittest.TestCase):
    def checks(self, backend="DFS", observed=None, base=None, node=None, meta=None, system="Linux"):
        observed = observed or mount()
        return target.target_checks(system, backend, observed, base or observed, node, meta)

    def test_observed_product(self):
        self.assertTrue(all(self.checks(node=process("afs-node"), meta=process("afs-meta")).values()))

    def test_ext4_cannot_be_product(self):
        self.assertFalse(self.checks(observed=mount("/dev/vda1", "ext4"))["observed-target-backend"])

    def test_other_fuse_cannot_be_product(self):
        self.assertFalse(self.checks(observed=mount("other"))["observed-target-backend"])

    def test_backend_mismatch(self):
        self.assertFalse(self.checks(backend="OwnerFs")["observed-target-backend"])

    def test_reference_ext4(self):
        self.assertTrue(all(self.checks(backend="reference", observed=mount("/dev/vda1", "ext4")).values()))

    def test_unknown_label(self):
        self.assertFalse(self.checks(backend="something")["observed-target-backend"])

    def test_no_product_process(self):
        self.assertFalse(self.checks()["product-process-identity"])

    def test_wrong_process(self):
        self.assertFalse(self.checks(node=process("python3"), meta=process("afs-meta"))["product-process-identity"])

    def test_nested_mount(self):
        self.assertFalse(self.checks(base=mount(target="/mount/nested"))["same-fixture-filesystem"])

    def test_non_linux(self):
        self.assertFalse(self.checks(system="Darwin")["linux-runtime"])

    def test_invalid_mount_json(self):
        self.assertFalse(self.checks(observed={"returncode": 0, "stdout": "broken"})["observed-target-backend"])


if __name__ == "__main__":
    unittest.main()
