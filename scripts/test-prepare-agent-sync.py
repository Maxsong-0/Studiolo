#!/usr/bin/env python3
"""验证自动同步配置可以独立运行、保留路径边界且不会误覆盖既有任务。"""
import importlib.util
import os
from pathlib import Path
import plistlib
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("prepare_sync", Path(__file__).with_name("prepare-agent-sync.py"))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PrepareTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.source = self.root / "source"
        self.source.mkdir()
        (self.source / "sync-agent-vault.sh").write_text("original script")
        (self.source / "replica_client.py").write_text("original client")
        self.vault = self.root / "Owner's & Vault"
        self.vault.mkdir()
        self.output = self.root / "runtime's & bundle"

    def prepare(self):
        with patch.object(module, "__file__", str(self.source / "prepare-agent-sync.py")):
            return module.prepare(self.output, self.vault, "owner@host", "/home/owner/replica", sys.executable)

    def test_runtime_is_frozen_and_paths_are_plist_values_not_shell_code(self):
        plist = self.prepare()
        config = plistlib.loads(plist.read_bytes())
        self.assertEqual(config["EnvironmentVariables"]["STUDIOLO_VAULT"], str(self.vault))
        self.assertEqual(config["ProgramArguments"], ["/bin/bash", str(self.output / "scripts/sync-agent-vault.sh")])
        self.assertEqual(config["StartInterval"], 60)
        self.assertEqual(config["EnvironmentVariables"]["STUDIOLO_PYTHON"], sys.executable)
        (self.source / "replica_client.py").write_text("later branch changed")
        self.assertEqual((self.output / "scripts/replica_client.py").read_text(), "original client")
        self.assertEqual(os.stat(self.output).st_mode & 0o777, 0o700)
        self.assertEqual(os.stat(plist).st_mode & 0o777, 0o600)

    def test_existing_output_is_never_overwritten(self):
        self.output.mkdir()
        sentinel = self.output / "keep"
        sentinel.write_text("unchanged")
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertEqual(sentinel.read_text(), "unchanged")

    def test_symlink_runtime_and_missing_source_fail_without_writes(self):
        self.output.symlink_to(self.vault, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertEqual(list(self.vault.iterdir()), [])
        self.output.unlink()
        (self.source / "replica_client.py").unlink()
        with self.assertRaises(ValueError):
            self.prepare()
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
