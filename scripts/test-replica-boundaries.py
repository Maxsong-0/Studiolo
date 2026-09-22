#!/usr/bin/env python3
"""不可信 tar/清单、路径和发布锁的真实 IO 边界回归。"""
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tarfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
from replica_archive import Reader, pax_path, receive
from replica_common import (Invalid, digest, encode, manifest, open_root, path_ok,
                            read_file, require)
from replica_store import Store

spec = importlib.util.spec_from_file_location("receiver_tests", Path(__file__).with_name("test-replica-receiver.py"))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class BoundaryTests(unittest.TestCase):
    setUp = fixtures.ReceiverTests.setUp
    payload = fixtures.ReceiverTests.payload
    run_receive = fixtures.ReceiverTests.run_receive
    state = fixtures.ReceiverTests.state
    def test_archive_paths_types_and_duplicate_members_rejected(self):
        cases = (("../../escape", tarfile.REGTYPE), ("/tmp/escape", tarfile.REGTYPE),
                 ("vault/项目/Studiolo/.hidden", tarfile.REGTYPE),
                 ("vault/项目/Studiolo/link", tarfile.SYMTYPE),
                 ("vault/项目/Studiolo/hard", tarfile.LNKTYPE),
                 ("vault/项目/Studiolo/fifo", tarfile.FIFOTYPE),
                 ("manifest.json", tarfile.REGTYPE),
                 ("vault/项目/Studiolo/encoded%2e", tarfile.REGTYPE))
        for name, kind in cases:
            with self.subTest(path=name, kind=kind):
                def edit(member, body):
                    if member.name.endswith("note.md"):
                        member.name, member.type = name, kind
                        member.linkname = "../../escape"
                    return member, body
                self.run_receive(fixtures.rewrite(self.payload(), edit), 1)
                self.assertIsNone(self.state()["receipt"])
        self.assertFalse((self.root / "escape").exists())

    def test_manifest_limits_duplicate_keys_and_collision_rejected(self):
        value = json.loads(self.payload_manifest())
        variants = []
        duplicate = encode(value).replace(b'"schema_version":1', b'"schema_version":1,"schema_version":1')
        variants.append(duplicate)
        for update in (lambda v: v.update(allow=["项目/Studiolo"]),
                       lambda v: v["entries"][-1].update(bytes=129 * 1024 * 1024),
                       lambda v: v["entries"][-1].update(sha256="A" * 64),
                       lambda v: v["entries"][-1].update(bytes=True),
                       lambda v: v["entries"].reverse(),
                       lambda v: v["entries"][-1].update(path="项目/Studiolo/NFD-e\u0301.md")):
            copy_value = json.loads(encode(value))
            update(copy_value)
            variants.append(encode(copy_value))
        collision = {"schema_version": 1, "allow": value["allow"], "entries": [
            {"path": "项目/Studiolo/A.md", "kind": "file", "bytes": 0, "sha256": digest(b"")},
            {"path": "项目/Studiolo/a.md", "kind": "file", "bytes": 0, "sha256": digest(b"")} ]}
        variants.append(encode(collision))
        for raw in variants:
            with self.subTest(raw=raw[:40]):
                payload = fixtures.rewrite(self.payload(), lambda m, b: (m, raw) if m.name == "manifest.json" else (m, b))
                self.run_receive(payload, 1)

    def payload_manifest(self):
        with tarfile.open(fileobj=io.BytesIO(self.payload())) as tar:
            return tar.extractfile("manifest.json").read()

    def test_pax_metadata_size_and_raw_stream_limit_enforced_before_allocating(self):
        header = tarfile.TarInfo("meta")
        header.type = tarfile.XHDTYPE
        header.size = 8 * 1024 * 1024
        self.run_receive(header.tobuf() + bytes(1024), 1)
        with patch("replica_archive.MAX_WIRE", 16), self.assertRaises(Invalid):
            Reader(io.BytesIO(bytes(32))).read(32)
        for raw in (b"11 path=x\n", b"15 linkpath=x\n", b"20 path=../escape\n", b"0 path=x\n"):
            with self.subTest(raw=raw), self.assertRaises(Invalid):
                pax_path(raw)

    def test_archive_header_count_and_trailing_data_are_bounded(self):
        root = open_root(str(self.root / "stage"), create=True, private=True)
        try:
            with patch("replica_archive.MAX_HEADERS", 1), self.assertRaises(Invalid):
                receive(io.BytesIO(self.payload()), root)
        finally:
            os.close(root)
        self.run_receive(self.payload() + b"nonzero trailing bytes", 1)

    def test_non_nfc_and_forbidden_encodings_match_vault_path(self):
        for path in ("notes/e\u0301.md", "notes/%2fsecret", "notes/%5csecret", "notes/%00", "notes/%2e",
                     "notes/back\\slash", "notes/\x01control", "notes/\u1ab0mark", "C:/secret", "../escape"):
            with self.subTest(path=path), self.assertRaises(Invalid):
                path_ok(path)

    def test_replaced_lock_binding_rejects_state_mutation(self):
        store = Store(str(self.server))
        try:
            lock = self.server / "sync.lock"
            lock.rename(self.server / "original.lock")
            lock.write_bytes(b"")
            lock.chmod(0o600)
            with self.assertRaises(Invalid):
                store.failed("transfer_failed")
            self.assertFalse((self.server / "control/state.json").exists())
        finally:
            store.close()

    def test_state_symlink_hardlink_and_duplicate_json_rejected(self):
        self.run_receive()
        state = self.server / "control/state.json"
        old = state.read_bytes()
        state.unlink()
        state.symlink_to(self.root / "outside")
        self.run_receive(code=1)
        self.assertFalse((self.root / "outside").exists())
        state.unlink()
        state.write_bytes(old)
        state.chmod(0o600)
        os.link(state, self.root / "hard")
        self.run_receive(code=1)
        (self.root / "hard").unlink()
        state.write_bytes(old.replace(b'"schema_version":1', b'"schema_version":1,"schema_version":1'))
        self.run_receive(code=1)

    def test_cross_device_file_rejected(self):
        root = open_root(str(self.project))
        actual = os.fstat
        def foreign(fd):
            st = actual(fd)
            if fd == root:
                return st
            data = list(st)
            data[2] = st.st_dev + 1
            return os.stat_result(data)
        try:
            with patch("replica_common.os.fstat", side_effect=foreign), self.assertRaises(Invalid):
                read_file(root, "note.md", 1024, private=False)
        finally:
            os.close(root)

    def test_total_markdown_budget_and_utf8_rejected_at_receiver(self):
        (self.project / "note.md").write_bytes(b"one!")
        (self.project / "second.md").write_bytes(b"two!")
        raw_manifest = self.payload_manifest()
        with patch("replica_common.MAX_NOTES_TOTAL", 6), self.assertRaises(Invalid):
            manifest(raw_manifest)
        def invalid_utf8(member, body):
            if member.name.endswith("note.md"):
                body = b"\xff"
            elif member.name == "manifest.json":
                value = json.loads(body)
                for entry in value["entries"]:
                    if entry["path"].endswith("note.md"):
                        entry.update(bytes=1, sha256=digest(b"\xff"))
                body = encode(value)
            return member, body
        self.run_receive(fixtures.rewrite(self.payload(), invalid_utf8), 1)
        self.assertIsNone(self.state()["receipt"])

    def test_python_and_rust_replica_limits_agree(self):
        """捕获／接收端（Python）与 agent 验证端（Rust）的副本上限必须逐项相等。

        两端各有一份常量；只改一端，Mac 能推上去的副本会在服务器上被拒（或反之），
        而各自的测试都是绿的。2026-09-22 T-108 抬总量上限时补上这条对账。
        """
        import re
        import replica_common
        source = (Path(__file__).resolve().parent.parent
                  / "crates/supply/src/replica/protocol.rs").read_text(encoding="utf-8")

        def rust(name):
            match = re.search(rf"const {name}: \w+ = ([0-9_* ]+);", source)
            self.assertIsNotNone(match, name)
            value = 1
            for factor in match.group(1).split("*"):
                value *= int(factor.strip().replace("_", ""))
            return value

        pairs = {"MANIFEST_LIMIT": "MAX_MANIFEST", "FILE_LIMIT": "MAX_FILE",
                 "NOTES_TOTAL_LIMIT": "MAX_NOTES_TOTAL", "TOTAL_LIMIT": "MAX_TOTAL",
                 "ENTRY_LIMIT": "MAX_ENTRIES"}
        mismatched = {r: (rust(r), getattr(replica_common, p)) for r, p in pairs.items()
                      if rust(r) != getattr(replica_common, p)}
        self.assertEqual(mismatched, {}, "Rust 与 Python 的副本上限不一致（Rust 值, Python 值）")


if __name__ == "__main__":
    suite = unittest.TestSuite(BoundaryTests(name) for name in BoundaryTests.__dict__ if name.startswith("test_"))
    result = unittest.TextTestRunner().run(suite)
    sys.exit(not result.wasSuccessful())
