import tempfile
import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from go_sdk_artifact_consumer import MODULE, assert_no_local_dependency, consumer_env, latest_version, write_consumer


class GoSdkArtifactConsumerTest(unittest.TestCase):
    def test_latest_version_reads_last_proxy_entry(self):
        with tempfile.TemporaryDirectory() as tmp:
            proxy = Path(tmp)
            version_dir = proxy / MODULE / "@v"
            version_dir.mkdir(parents=True)
            (version_dir / "list").write_text("v0.1.0-rc.aaa\nv0.1.0-rc.bbb\n", encoding="utf-8")

            self.assertEqual(latest_version(proxy), "v0.1.0-rc.bbb")

    def test_generated_consumer_uses_versioned_module_without_replace(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            write_consumer(app, "v0.1.0-rc.test")

            assert_no_local_dependency(app)
            go_mod = (app / "go.mod").read_text(encoding="utf-8")
            self.assertIn(f"require {MODULE} v0.1.0-rc.test", go_mod)
            self.assertNotIn("replace ", go_mod)
            self.assertNotIn("../", go_mod)
            main_go = (app / "main.go").read_text(encoding="utf-8")
            self.assertIn("func verifyHash", main_go)
            self.assertIn("verifyStableErrors(ctx, client, prefix)", main_go)
            self.assertNotIn('"artifact-short-buffer"', main_go)

    def test_local_dependency_markers_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            app = Path(tmp)
            (app / "go.mod").write_text(
                f"module bad\nrequire {MODULE} v0.1.0\nreplace {MODULE} => ../sdk/go\n",
                encoding="utf-8",
            )

            with self.assertRaises(RuntimeError):
                assert_no_local_dependency(app)

    def test_consumer_env_forces_isolated_go_caches(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            proxy = root / "proxy"
            proxy.mkdir()
            env = consumer_env(
                {
                    "PATH": "/usr/bin",
                    "GOCACHE": "/host/cache",
                    "GOMODCACHE": "/host/modcache",
                },
                proxy,
                "http://127.0.0.1:26200",
                False,
                root,
            )

            self.assertEqual(env["GOCACHE"], str(root / "gocache"))
            self.assertEqual(env["GOMODCACHE"], str(root / "gomodcache"))
            self.assertTrue(env["GOPROXY"].startswith(proxy.as_uri()))
            self.assertEqual(env["DMS_SHARED_MEMORY"], "false")
            self.assertEqual(env["GONOSUMDB"], MODULE)
            self.assertNotIn("GOSUMDB", env)


if __name__ == "__main__":
    unittest.main()
