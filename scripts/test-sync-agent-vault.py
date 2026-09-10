#!/usr/bin/env python3
"""源捕获与真实接收器的本地 SSH 替身测试，不连接服务器。"""
import fcntl
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
from replica_capture import capture
from replica_client import accept_response
from replica_common import Invalid, digest, encode

SCRIPT = Path(__file__).absolute().with_name("sync-agent-vault.sh")


class SyncAgentVaultTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir="/private/tmp" if sys.platform == "darwin" else None)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.vault = self.root / "Owner's Vault"
        self.project = self.vault / "项目/Studiolo"
        self.project.mkdir(parents=True)
        (self.vault / "_规则").mkdir()
        (self.project / "note.md").write_text("private fixture text", encoding="utf-8")
        self.server = self.root / "server's copy"
        self.state = self.root / "client-state"
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        self.log = self.root / "calls.jsonl"
        stub = bin_dir / "ssh"
        stub.write_text("#!/usr/bin/env python3\nimport os, sys, json, subprocess\n"
                        "with open(os.environ['STUDIOLO_TEST_LOG'], 'a') as f:\n"
                        "    f.write(json.dumps(sys.argv[:-1]) + '\\n')\n"
                        "sys.exit(subprocess.call(['/bin/sh', '-c', sys.argv[-1]]))\n")
        stub.chmod(0o755)
        self.env = dict(os.environ, PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
                        STUDIOLO_VAULT=str(self.vault), STUDIOLO_REMOTE_REPLICA=str(self.server),
                        STUDIOLO_SYNC_STATE_DIR=str(self.state), STUDIOLO_TEST_LOG=str(self.log),
                        STUDIOLO_PYTHON=sys.executable, PYTHONDONTWRITEBYTECODE="1")

    def run_sync(self, args=(), code=0):
        result = subprocess.run(["bash", str(SCRIPT), *args], env=self.env,
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode, code, result.stderr)
        self.assertNotIn("private fixture text", result.stdout + result.stderr)
        return result

    def test_dry_run_is_entirely_local_and_content_free(self):
        value = json.loads(self.run_sync(["--dry-run"]).stdout)
        self.assertEqual(value["status"], "dry_run")
        self.assertEqual(value["files"], 1)
        self.assertFalse(self.log.exists())
        self.assertFalse(self.server.exists())

    def test_real_run_transfers_verified_generation_in_one_ssh_call(self):
        result = json.loads(self.run_sync().stdout)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(calls), 1)
        self.assertIn("BatchMode=yes", calls[0])
        self.assertIn("StrictHostKeyChecking=yes", calls[0])
        self.assertEqual(result["status"], "ready")
        path = self.server / "releases" / result["receipt"]["generation"] / "vault/项目/Studiolo/note.md"
        self.assertEqual(path.read_text(), "private fixture text")
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_shell_metacharacters_in_remote_path_are_quoted(self):
        marker = self.root / "injected"
        self.server = self.root / "server;$(touch injected) 'copy'"
        self.env["STUDIOLO_REMOTE_REPLICA"] = str(self.server)
        self.run_sync()
        self.assertTrue((self.server / "control/state.json").is_file())
        self.assertFalse(marker.exists())
        self.assertFalse(SCRIPT.parent.parent.joinpath("injected").exists())

    def test_extra_argument_rejected_before_remote_access(self):
        self.run_sync(["--dry-run", "unexpected"], 2)
        self.assertFalse(self.log.exists())

    def test_host_option_injection_is_rejected(self):
        self.env["STUDIOLO_HOST"] = "-oProxyCommand=touch"
        self.run_sync(code=1)
        self.assertFalse(self.log.exists())

    def test_excluded_hidden_subtree_is_not_traversed(self):
        hidden = self.project / ".ignored"
        hidden.mkdir()
        (hidden / "link").symlink_to(self.root)
        (self.vault / "outside.md").write_text("outside")
        value = json.loads(self.run_sync(["--dry-run"]).stdout)
        self.assertEqual(value["files"], 1)

    def test_visible_symlink_rejected_before_remote_access(self):
        (self.project / "link").symlink_to(self.root)
        self.run_sync(["--dry-run"], 1)
        self.assertFalse(self.log.exists())

    def test_visible_hard_link_rejected(self):
        os.link(self.project / "note.md", self.project / "second.md")
        self.run_sync(["--dry-run"], 1)

    def test_visible_fifo_rejected_without_blocking(self):
        os.mkfifo(self.project / "pipe")
        self.run_sync(["--dry-run"], 1)

    def test_middle_parent_symlink_rejected(self):
        alias = self.root / "alias"
        alias.symlink_to(self.vault.parent, target_is_directory=True)
        self.env["STUDIOLO_VAULT"] = str(alias / self.vault.name)
        self.run_sync(["--dry-run"], 1)

    def test_allow_parent_symlink_rejected(self):
        (self.vault / "项目").rename(self.vault / "real-project")
        (self.vault / "项目").symlink_to(self.vault / "real-project")
        self.run_sync(["--dry-run"], 1)

    def test_source_group_write_rejected(self):
        (self.project / "note.md").chmod(0o660)
        self.run_sync(["--dry-run"], 1)

    def test_source_change_during_capture_rejects_entire_batch(self):
        def mutate():
            (self.project / "note.md").write_text("changed")
        with self.assertRaises(Invalid):
            capture(str(self.vault), between_scans=mutate)

    def test_source_delete_during_capture_rejected(self):
        with self.assertRaises(Invalid):
            capture(str(self.vault), between_scans=lambda: (self.project / "note.md").unlink())

    def test_source_added_empty_directory_during_capture_rejected(self):
        with self.assertRaises(Invalid):
            capture(str(self.vault), between_scans=lambda: (self.project / "empty").mkdir())

    def test_source_same_bytes_replaced_inode_rejected(self):
        def replace():
            note = self.project / "note.md"
            raw = note.read_bytes()
            note.unlink()
            note.write_bytes(raw)
        with self.assertRaises(Invalid):
            capture(str(self.vault), between_scans=replace)

    def test_source_lock_prevents_concurrent_capture(self):
        self.state.mkdir(mode=0o700)
        host = self.env.get("STUDIOLO_HOST", "maxsong@192.168.100.169")
        key = digest(encode([str(self.vault), host, str(self.server)]))[:32]
        with open(self.state / ("sync-" + key + ".lock"), "wb") as locked:
            os.fchmod(locked.fileno(), 0o600)
            fcntl.flock(locked, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.run_sync(["--dry-run"], 1)
        self.assertFalse(self.log.exists())

    def test_all_names_enumeration_is_bounded_including_hidden(self):
        for n in range(6):
            (self.project / (".hidden" + str(n))).touch()
        with patch("replica_capture.MAX_ENTRIES", 5), self.assertRaises(Invalid):
            capture(str(self.vault))

    def test_total_markdown_budget_and_utf8_are_checked_at_source(self):
        (self.project / "note.md").write_bytes(b"one!")
        (self.project / "second.md").write_bytes(b"two!")
        with patch("replica_capture.MAX_NOTES_TOTAL", 6), self.assertRaises(Invalid):
            capture(str(self.vault))
        (self.project / "note.md").write_bytes(b"\xff")
        self.run_sync(["--dry-run"], 1)
        self.assertFalse(self.log.exists())

    def test_remote_ack_must_match_this_capture_without_extra_fields(self):
        good = {"status": "ready", "reused": False, "receipt": {
            "generation": "a" * 32, "source_digest": "b" * 64,
            "source_observed_at_ms": 100, "received_at_ms": 101}}
        self.assertEqual(accept_response(good, "b" * 64, 100), good)
        for change in (
            lambda v: v.update(unexpected="private fixture text"),
            lambda v: v.update(reused="yes"),
            lambda v: v["receipt"].update(source_observed_at_ms=99),
            lambda v: v["receipt"].update(generation="../../escape"),
            lambda v: v["receipt"].update(received_at_ms=True),
        ):
            value = json.loads(json.dumps(good))
            change(value)
            with self.assertRaises(Invalid):
                accept_response(value, "b" * 64, 100)


if __name__ == "__main__":
    unittest.main()
