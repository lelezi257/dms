import unittest

from candidate_registry import index_entry


class RegistryContractTest(unittest.TestCase):
    def test_renamed_optional_dependencies_keep_wire_semantics(self):
        manifest = {
            "package": {"name": "dms-client", "version": "0.1.0"},
            "dependencies": {"alias": {"version": "1", "package": "actual",
                                       "optional": True, "default-features": False}},
            "features": {"extra": ["dep:alias"]},
            "target": {"cfg(unix)": {"dependencies": {"libc": "0.2"}}},
        }
        entry = index_entry(manifest, b"payload")
        self.assertEqual(entry["deps"][0]["package"], "actual")
        self.assertFalse(entry["deps"][0]["default_features"])
        self.assertEqual(entry["deps"][1]["target"], "cfg(unix)")
        self.assertEqual(entry["deps"][0]["registry"],
                         "https://github.com/rust-lang/crates.io-index")
        self.assertEqual(entry["features2"], {"extra": ["dep:alias"]})
        self.assertEqual(len(entry["cksum"]), 64)

    def test_path_dependency_is_not_release_registry_dependency(self):
        for key in ("path", "git"):
            with self.subTest(key=key), self.assertRaises(ValueError):
                index_entry({"package": {"name": "dms-client", "version": "0.1.0"},
                             "dependencies": {"dms-common": {"version": "0.1", key: "x"}}}, b"")


if __name__ == "__main__":
    unittest.main()
