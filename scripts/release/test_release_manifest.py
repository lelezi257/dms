import json
from pathlib import Path
import subprocess
import tempfile
import unittest

import release_manifest


class ReleaseManifestTests(unittest.TestCase):
    def test_assets_use_file_names_and_hashes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(["git", "init", "-q", root], check=True)
            subprocess.run(["git", "-C", root, "config", "user.email", "test@example.invalid"], check=True)
            subprocess.run(["git", "-C", root, "config", "user.name", "test"], check=True)
            (root / "tracked").write_text("source")
            subprocess.run(["git", "-C", root, "add", "tracked"], check=True)
            subprocess.run(["git", "-C", root, "commit", "-qm", "fixture"], check=True)
            asset = root / "dms-server-0.1.0-linux-aarch64.tar.gz"
            asset.write_bytes(b"artifact")
            result = release_manifest.build_manifest(root, "v0.1.0", [asset], "example/dms")
            self.assertTrue(result["remote_publish"])
            self.assertEqual(result["assets"][0]["name"], asset.name)
            self.assertNotIn(str(root), json.dumps(result))


if __name__ == "__main__":
    unittest.main()
