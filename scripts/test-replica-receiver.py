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
from replica_archive import MAX_DEADLINE, MIN_RATE, transfer_deadline
from replica_capture import archive, capture
from replica_common import ALLOW, digest, encode
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
        # 夹具按默认权限建目录，而被测的捕获端与接收器按设计拒绝组／其他可写的节点。
        # Ubuntu 普通用户默认 umask 002 会让夹具目录变成 0775 而被拒；测试自己钉住前提，不依赖环境。
        self.addCleanup(os.umask, os.umask(0o022))
        self.temp = tempfile.TemporaryDirectory(dir="/private/tmp" if sys.platform == "darwin" else None)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.vault = self.root / "vault"
        self.project = self.vault / "项目/Studiolo"
        # 捕获端对 ALLOW 里每个目录都 open_dir，缺一个即整批失败；
        # 从 ALLOW 派生而不是逐个硬编码，避免扩白名单时漏建。
        for allow in ALLOW:
            (self.vault / allow).mkdir(parents=True, exist_ok=True)
        (self.project / "note.md").write_text("private fixture text")
        (self.project / "empty").mkdir()
        self.server = self.root / "server"

    def payload(self, base=None):
        with archive(capture(str(self.vault)), base) as stream:
            return stream.read()

    def run_receive(self, payload=None, code=0, root=None, deadline="120"):
        if payload is None:
            payload = self.payload()
        result = subprocess.run([sys.executable, "-B", str(RECEIVER), "--root", str(root or self.server),
                                 "--deadline", deadline],
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

    def base_of(self):
        """源端基线：上一轮被服务器确认的清单摘要与 expected。"""
        raw, _, expected, _ = capture(str(self.vault))
        return digest(raw), expected

    @staticmethod
    def members(payload):
        with tarfile.open(fileobj=io.BytesIO(payload)) as tar:
            return [member.name for member in tar]

    def request_rewrite(self, payload, **fields):
        def edit(member, body):
            if member.name == "request.json":
                value = json.loads(body)
                value.update(fields)
                body = encode(value)
            return member, body
        return rewrite(payload, edit)

    def releases(self):
        return sorted(p.name for p in (self.server / "releases").iterdir())

    def test_confirm_sends_only_manifest_and_refreshes_receipt_without_new_generation(self):
        first = self.run_receive()
        releases = self.releases()
        payload = self.payload(self.base_of())
        self.assertEqual(self.members(payload), ["manifest.json", "request.json"])
        second = self.run_receive(payload)
        self.assertTrue(second["reused"])
        self.assertEqual(second["receipt"]["generation"], first["receipt"]["generation"])
        self.assertGreaterEqual(second["receipt"]["source_observed_at_ms"], first["receipt"]["source_observed_at_ms"])
        self.assertEqual(self.state()["status"], "ready")
        self.assertEqual(self.releases(), releases)
        self.assertFalse(list(self.server.glob(".stage-*")))

    def test_delta_sends_only_changed_files_and_copies_the_rest_from_current_generation(self):
        attachment = b"\x00attachment" * 100
        (self.project / "keep.bin").write_bytes(attachment)
        self.run_receive()
        base = self.base_of()
        (self.project / "note.md").write_text("second version")
        (self.project / "added.md").write_text("added")
        (self.project / "empty").rmdir()
        payload = self.payload(base)
        names = self.members(payload)
        self.assertIn("vault/项目/Studiolo/note.md", names)
        self.assertIn("vault/项目/Studiolo/added.md", names)
        self.assertNotIn("vault/项目/Studiolo/keep.bin", names)
        self.assertNotIn(b"attachment", payload)
        second = self.run_receive(payload)
        self.assertFalse(second["reused"])
        new = self.current_vault() / "项目/Studiolo"
        self.assertEqual((new / "keep.bin").read_bytes(), attachment)
        self.assertEqual((new / "keep.bin").stat().st_mode & 0o777, 0o600)
        self.assertEqual((new / "note.md").read_text(), "second version")
        self.assertEqual((new / "added.md").read_text(), "added")
        self.assertFalse((new / "empty").exists())
        self.assertEqual(len(self.releases()), 2)

    def test_base_that_is_not_the_current_generation_is_refused_without_touching_state(self):
        empty = self.payload(self.base_of())
        self.assertEqual(self.run_receive(empty), {"status": "base_mismatch"})
        self.assertFalse((self.server / "control/state.json").exists())
        self.run_receive()
        before = self.state()
        base = self.base_of()
        (self.project / "note.md").write_text("changed")
        self.assertEqual(self.run_receive(self.payload(("0" * 64, base[1]))), {"status": "base_mismatch"})
        self.assertEqual(self.state(), before)
        self.assertEqual(len(self.releases()), 1)

    def test_corrupt_current_generation_fails_confirm_and_is_recorded_until_full_repair(self):
        first = self.run_receive()
        (self.current_vault() / "项目/Studiolo/note.md").write_text("private fixture edit")
        self.assertEqual(self.run_receive(self.payload(self.base_of())), {"status": "base_mismatch"})
        self.assertEqual(self.state()["status"], "failed")
        self.assertEqual(self.state()["last_error"], "validation_failed")
        self.assertEqual(self.state()["receipt"], first["receipt"])
        self.run_receive()
        self.assertEqual(self.state()["status"], "ready")
        self.assertEqual((self.current_vault() / "项目/Studiolo/note.md").read_text(), "private fixture text")

    def test_corrupt_unchanged_file_fails_delta_copy_and_is_recorded(self):
        (self.project / "keep.md").write_text("keep this")
        first = self.run_receive()
        base = self.base_of()
        (self.current_vault() / "项目/Studiolo/keep.md").write_text("keep that")
        (self.project / "note.md").write_text("second version")
        self.assertEqual(self.run_receive(self.payload(base)), {"status": "base_mismatch"})
        self.assertEqual(self.state()["last_error"], "validation_failed")
        self.assertEqual(self.state()["receipt"], first["receipt"])
        self.assertEqual(len(self.releases()), 1)
        self.assertFalse(list(self.server.glob(".stage-*")))

    def test_delta_and_confirm_cannot_bend_their_shape(self):
        self.run_receive()
        old = self.state()["receipt"]
        base = self.base_of()
        unchanged = self.payload()
        (self.project / "note.md").write_text("second version")
        delta = self.payload(base)
        full = self.payload()
        cases = {
            "增量漏发变化文件": rewrite(delta, lambda m, b: (None, None) if m.name.endswith("note.md") else (m, b)),
            "增量漏发目录": rewrite(delta, lambda m, b: (None, None) if m.name.endswith("/empty") else (m, b)),
            # 基线确为现代次、清单也未变，却夹带正文：确认请求只许有清单与请求两项。
            "确认夹带正文": self.request_rewrite(unchanged, base_digest=base[0]),
            "旧版请求": self.request_rewrite(full, schema_version=1),
            "基线摘要非法": self.request_rewrite(full, base_digest="A" * 64),
        }
        for name, payload in cases.items():
            with self.subTest(case=name):
                self.run_receive(payload, 1)
                self.assertEqual(self.state()["receipt"], old)

    def test_deadline_is_bounded_before_touching_the_store(self):
        for deadline in ("119", "0", str(MAX_DEADLINE + 1)):
            with self.subTest(deadline=deadline):
                self.run_receive(code=1, deadline=deadline)
                self.assertFalse(self.server.exists())
        self.run_receive(deadline=str(MAX_DEADLINE))
        self.assertEqual([transfer_deadline(n) for n in (0, 1, MIN_RATE, MIN_RATE + 1)], [120, 121, 121, 122])
        self.assertEqual(transfer_deadline(219_321_960), 120 + 210)

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
                raw = rewrite(self.payload(), lambda m, b: (m, encode({"schema_version": 2,
                              "source_observed_at_ms": when, "base_digest": None}))
                              if m.name == "request.json" else (m, b))
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
        command = [sys.executable, "-B", str(RECEIVER), "--root", str(self.server), "--deadline", "120"]
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
