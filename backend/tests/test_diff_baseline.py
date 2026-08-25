import tempfile
import unittest
from pathlib import Path

import diff_baseline


class TreeSha256Tests(unittest.TestCase):
    def test_line_endings_do_not_change_hash(self):
        original_backend_root = diff_baseline.BACKEND_ROOT
        try:
            with tempfile.TemporaryDirectory() as temporary_directory:
                backend_root = Path(temporary_directory)
                data_root = backend_root / "data" / "version"
                data_root.mkdir(parents=True)
                data_file = data_root / "skills.toml"
                diff_baseline.BACKEND_ROOT = backend_root

                data_file.write_bytes(b"name = \"test\"\nvalue = 1\n")
                lf_hash = diff_baseline.tree_sha256([data_root])

                data_file.write_bytes(b"name = \"test\"\r\nvalue = 1\r\n")
                crlf_hash = diff_baseline.tree_sha256([data_root])

                self.assertEqual(lf_hash, crlf_hash)
        finally:
            diff_baseline.BACKEND_ROOT = original_backend_root


if __name__ == "__main__":
    unittest.main()
