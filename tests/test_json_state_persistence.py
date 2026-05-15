import json
import os
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import codoxear.server as server


class TestJsonStatePersistence(unittest.TestCase):
    def test_write_json_state_atomic_creates_parent_and_formats_sorted_json(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "state" / "sample.json"

            server._write_json_state_atomic(path, {"z": 1, "a": 2})

            self.assertEqual(
                path.read_text(encoding="utf-8"),
                '{\n  "a": 2,\n  "z": 1\n}\n',
            )

    def test_write_json_state_atomic_can_preserve_object_key_order(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "sample.json"

            server._write_json_state_atomic(path, {"z": 1, "a": 2}, sort_keys=False)

            self.assertEqual(
                list(json.loads(path.read_text(encoding="utf-8")).keys()),
                ["z", "a"],
            )

    def test_write_json_state_atomic_removes_temp_file_on_replace_failure(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "sample.json"

            def fail_replace(
                src: str | os.PathLike[str], dst: str | os.PathLike[str]
            ) -> None:
                raise OSError("replace failed")

            with patch.object(server.os, "replace", side_effect=fail_replace):
                with self.assertRaisesRegex(OSError, "replace failed"):
                    server._write_json_state_atomic(path, {"a": 1})

            self.assertFalse(path.exists())
            self.assertEqual(list(Path(td).glob("*.tmp")), [])


if __name__ == "__main__":
    unittest.main()
