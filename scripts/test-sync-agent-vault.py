#!/usr/bin/env python3
"""在临时虚构库与 ssh/rsync 替身上检查同步边界，不连接服务器。"""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class SyncAgentVaultTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.vault = self.root / "Owner's Vault"
        self.project = self.vault / "项目/Studiolo"
        self.project.mkdir(parents=True)
        (self.vault / "_规则").mkdir()
        (self.project / "note.md").write_text("fixture", encoding="utf-8")
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        self.log = self.root / "calls.jsonl"
        for name in ("ssh", "rsync"):
            stub = bin_dir / name
            stub.write_text(
                "#!/usr/bin/env python3\nimport os, sys, json\n"
                'with open(os.environ["STUDIOLO_TEST_LOG"], "a") as f:\n'
                '    f.write(json.dumps(sys.argv) + "\\n")\n',
                encoding="utf-8",
            )
            stub.chmod(0o755)
        self.env = dict(
            os.environ,
            PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
            STUDIOLO_VAULT=str(self.vault),
            STUDIOLO_TEST_LOG=str(self.log),
        )

    def run_sync(self, args, expected_code=0):
        script = Path(__file__).resolve().with_name("sync-agent-vault.sh")
        result = subprocess.run(
            ["bash", str(script), *args], env=self.env, capture_output=True, text=True
        )
        self.assertEqual(result.returncode, expected_code, result.stderr)
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []

    def test_dry_run_only_calls_rsync_with_dry_run(self):
        calls = self.run_sync(["--dry-run"])
        self.assertEqual(len(calls), 2)
        for call in calls:
            self.assertEqual(Path(call[0]).name, "rsync")
            self.assertIn("--dry-run", call)

    def test_real_run_prepares_and_restricts_permissions(self):
        calls = self.run_sync([])
        self.assertEqual(sum(Path(call[0]).name == "ssh" for call in calls), 4)
        self.assertEqual(sum(Path(call[0]).name == "rsync" for call in calls), 2)
        self.assertTrue(all("--dry-run" not in call for call in calls))

    def test_extra_argument_is_rejected_before_remote_access(self):
        self.assertEqual(self.run_sync(["--dry-run", "unexpected"], 2), [])

    def test_visible_symlink_is_rejected_before_remote_access(self):
        (self.project / "link").symlink_to(self.root)
        self.assertEqual(self.run_sync(["--dry-run"], 1), [])

    def test_excluded_hidden_subtree_is_not_traversed(self):
        hidden = self.project / ".ignored"
        hidden.mkdir()
        (hidden / "link").symlink_to(self.root)
        self.assertEqual(len(self.run_sync(["--dry-run"])), 2)


if __name__ == "__main__":
    unittest.main()
