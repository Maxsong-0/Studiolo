#!/usr/bin/env python3
"""用真实接收子进程验证完整批次发布和失败恢复。"""
import copy
import io
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest

sys.dont_write_bytecode = True
from replica_capture import archive, capture
from replica_common import encode
from replica_store import Store

RECEIVER = Path(__file__).absolute().with_name("replica_receiver.py")


def rewrite(raw, transform):
    out = io.BytesIO()
    with tarfile.open(fileobj=io.BytesIO(raw)) as src, tarfile.open(fileobj=out, mode="w") as dst:
        for member in src:
            body = src.extractfile(member).read() if member.isfile() else None
            member, body = transform(copy.copy(member), body)
            if member is not None:
                member.pax_headers = {}
                member.size = len(body) if body is not None else 0
                dst.addfile(member, io.BytesIO(body) if body is not None else None)
    return out.getvalue()


class ReceiverTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir="/private/tmp" if sys.platform == "darwin" else None)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.vault = self.root / "vault"
        self.project = self.vault / "项目/Studiolo"
        self.project.mkdir(parents=True)
        (self.vault / "_规则").mkdir()
        (self.project / "note.md").write_text("private fixture text")
        (self.project / "empty").mkdir()
        self.server = self.root / "server"

    def payload(self):
        with archive(capture(str(self.vault))) as stream:
            return stream.read()

    def run_receive(self, payload=None, code=0, root=None):
        if payload is None:
            payload = self.payload()
        result = subprocess.run([sys.executable, "-B", str(RECEIVER), "--root", str(root or self.server)],
                                input=payload, capture_output=True, timeout=20)
        self.assertEqual(result.returncode, code, result.stderr.decode())
        self.assertNotIn(b"private fixture text", result.stdout + result.stderr)
        self.assertNotIn(b"Traceback", result.stderr)
        return json.loads(result.stdout) if code == 0 else result

    def state(self):
        return json.loads((self.server / "control/state.json").read_text())

    def current_vault(self):
        return self.server / "releases" / self.state()["receipt"]["generation"] / "vault"

    def test_add_edit_delete_rename_and_empty_directories_publish_together(self):
        first = self.run_receive()
        old = self.current_vault()
        (self.project / "note.md").write_text("second version")
        (self.project / "added.md").write_text("added")
        (self.project / "empty").rename(self.project / "renamed")
        second = self.run_receive()
        self.assertNotEqual(first["receipt"]["generation"], second["receipt"]["generation"])
        new = self.current_vault() / "项目/Studiolo"
        self.assertEqual((new / "note.md").read_text(), "second version")
        self.assertTrue((new / "added.md").is_file())
        self.assertTrue((new / "renamed").is_dir())
        self.assertFalse((new / "empty").exists())
        self.assertEqual((old / "项目/Studiolo/note.md").read_text(), "private fixture text")
        (self.project / "note.md").unlink()
        self.run_receive()
        self.assertFalse((self.current_vault() / "项目/Studiolo/note.md").exists())
        self.assertEqual(len(list((self.server / "releases").iterdir())), 2)

    def test_same_digest_reuses_fully_validated_generation_and_refreshes_receipt(self):
        first = self.run_receive()
        second = self.run_receive()
        self.assertTrue(second["reused"])
        self.assertEqual(first["receipt"]["generation"], second["receipt"]["generation"])
        self.assertGreaterEqual(second["receipt"]["source_observed_at_ms"], first["receipt"]["source_observed_at_ms"])
        self.assertEqual(len(list((self.server / "releases").iterdir())), 1)

    def test_same_source_repairs_same_size_drift_using_new_generation(self):
        first = self.run_receive()
        (self.current_vault() / "项目/Studiolo/note.md").write_text("private fixture edit")
        second = self.run_receive()
        self.assertFalse(second["reused"])
        self.assertNotEqual(first["receipt"]["generation"], second["receipt"]["generation"])
        self.assertEqual((self.current_vault() / "项目/Studiolo/note.md").read_text(), "private fixture text")

    def test_corrupt_file_preserves_previous_receipt_and_recovers(self):
        self.run_receive()
        previous = self.state()["receipt"]
        raw = rewrite(self.payload(), lambda m, b: (m, b"x" * len(b) if m.name.endswith("note.md") else b))
        self.run_receive(raw, 1)
        self.assertEqual(self.state()["status"], "failed")
        self.assertEqual(self.state()["receipt"], previous)
        self.assertEqual(self.state()["last_error"], "validation_failed")
        self.run_receive()
        self.assertEqual(self.state()["status"], "ready")

    def test_truncated_stream_preserves_previous_and_removes_own_stage(self):
        self.run_receive()
        old = self.state()["receipt"]
        self.run_receive(self.payload()[:2048], 1)
        self.assertEqual(self.state()["receipt"], old)
        self.assertEqual(self.state()["last_error"], "transfer_failed")
        self.assertFalse(list(self.server.glob(".stage-*")))

    def test_checksum_error_is_bounded_failure_without_traceback(self):
        raw = bytearray(self.payload())
        raw[1] ^= 1
        self.run_receive(bytes(raw), 1)
        self.assertIsNone(self.state()["receipt"])

    def test_missing_directory_or_file_is_rejected(self):
        raw = rewrite(self.payload(), lambda m, b: (None, None) if m.name.endswith("note.md") else (m, b))
        self.run_receive(raw, 1)
        self.assertIsNone(self.state()["receipt"])

    def test_future_or_replayed_source_receipt_is_rejected(self):
        self.run_receive()
        old = self.state()["receipt"]
        for when in (1, old["received_at_ms"] + 120000):
            with self.subTest(when=when):
                raw = rewrite(self.payload(), lambda m, b: (m, encode({"schema_version": 1,
                              "source_observed_at_ms": when})) if m.name == "request.json" else (m, b))
                self.run_receive(raw, 1)
                self.assertEqual(self.state()["receipt"], old)

    def test_active_receiver_excludes_second_process(self):
        store = Store(str(self.server))
        try:
            self.run_receive(code=1)
            self.assertFalse((self.server / "control/state.json").exists())
        finally:
            store.close()
        self.run_receive()

    def test_parent_symlink_is_rejected_without_touching_destination(self):
        alias = self.root / "alias"
        alias.symlink_to(self.root)
        self.run_receive(root=alias / "server", code=1)
        self.assertFalse(self.server.exists())

    def test_existing_managed_permissions_and_link_rejected(self):
        self.server.mkdir(mode=0o755)
        # mkdir 的 mode 会被部署环境 umask 077 收紧；负测须显式制造 0755。
        self.server.chmod(0o755)
        self.run_receive(code=1)
        self.assertFalse((self.server / "control").exists())
        self.server.chmod(0o700)
        (self.server / "control").symlink_to(self.root)
        self.run_receive(code=1)
        self.assertFalse((self.root / "state.json").exists())

    def test_sigkill_stage_is_reclaimed_by_next_locked_receiver(self):
        command = [sys.executable, "-B", str(RECEIVER), "--root", str(self.server)]
        process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline and not list(self.server.glob(".stage-*")):
                time.sleep(0.01)
            self.assertTrue(list(self.server.glob(".stage-*")))
            process.send_signal(signal.SIGKILL)
            process.communicate(timeout=5)
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
        self.run_receive()
        self.assertFalse(list(self.server.glob(".stage-*")))

    def test_unsafe_orphan_stage_is_rejected_without_any_partial_cleanup(self):
        self.run_receive()
        stage = self.server / (".stage-" + "a" * 32)
        stage.mkdir(mode=0o700)
        safe = stage / "safe"
        safe.write_bytes(b"safe")
        safe.chmod(0o600)
        (stage / "unsafe").symlink_to(self.root)
        previous = self.state()
        self.run_receive(code=1)
        self.assertTrue(safe.is_file())
        self.assertEqual(self.state(), previous)

    def test_empty_allowlist_subtrees_are_valid(self):
        (self.project / "note.md").unlink()
        (self.project / "empty").rmdir()
        self.run_receive()
        self.assertTrue((self.current_vault() / "项目/Studiolo").is_dir())

    def test_maximum_logical_depth_roundtrip_and_extra_level_rejected(self):
        path = self.project
        for _ in range(61):
            path = path / "d"
            path.mkdir()
        (path / "leaf.md").write_text("deep")
        self.run_receive()
        (path / "deeper").mkdir()
        (path / "deeper/leaf.md").write_text("too deep")
        with self.assertRaises(ValueError):
            self.payload()


if __name__ == "__main__":
    unittest.main()
