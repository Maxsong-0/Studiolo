#!/usr/bin/env python3
"""仅原生 Linux 的虚构校验拒绝测试；通过 unittest discover 运行整组。"""
import hashlib
import contextlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import stat
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).absolute().parent))
from test_human_snapshot_fixture import Fixture, MODULE


class HumanSnapshotTests(Fixture):
    def test_non_delete_database_header_even_with_updated_hash_is_rejected(self):
        self.create()
        path = self.output / "auth/auth.sqlite3"
        raw = bytearray(path.read_bytes())
        raw[18:20] = b"\x02\x02"
        self.write(path, raw)
        self.manifest_edit(lambda value: value["files"]["auth/auth.sqlite3"].update(sha256=hashlib.sha256(raw).hexdigest()))
        self.verify(ok=False)

    def test_tamper_missing_extra_and_wrong_modes(self):
        self.create()
        path = self.output / "owner.json"
        original = path.read_bytes()
        for action in (lambda: self.write(path, b"tamper"), lambda: path.unlink(), lambda: path.chmod(0o644)):
            action()
            self.verify(ok=False)
            self.write(path, original)
        extra = self.output / "extra"
        self.write(extra, b"extra")
        self.verify(ok=False)
        extra.unlink()
        extra.mkdir(mode=0o700)
        self.verify(ok=False)
        extra.rmdir()
        binary = self.output / "release/bin/studiolo-local-server"
        binary.chmod(0o700)
        self.verify(ok=False)

    def test_output_links_and_special_files_rejected(self):
        self.create()
        target = self.output / "portable-factor.age"
        target.unlink()
        for kind in ("symlink", "hardlink", "fifo"):
            if kind == "symlink":
                target.symlink_to(self.portable)
            elif kind == "hardlink":
                os.link(self.portable, target)
            else:
                os.mkfifo(target, 0o600)
            self.verify(ok=False)
            target.unlink()

    def test_outer_manifest_strict_types_and_complete_marker(self):
        self.create()
        original = (self.output / "manifest.json").read_bytes()
        for key, value in (("schema_version", True), ("state", "incomplete"), ("unexpected", 1), ("snapshot_id", "bad/id")):
            self.write(self.output / "manifest.json", original)
            self.manifest_edit(lambda m: m.update({key: value}))
            self.verify(ok=False)
        self.write(self.output / "manifest.json", original.replace(b'"schema_version": 1', b'"schema_version": 1, "schema_version": 1'))
        self.verify(ok=False)
        self.write(self.output / "manifest.json", original)
        self.write(self.output / "INCOMPLETE", b"incomplete\n")
        self.verify(ok=False)

    def test_unknown_root_even_when_outer_manifest_matches(self):
        self.create()
        self.write(self.output / "unexpected", b"extra")
        def change(value):
            value["files"]["unexpected"] = {"size": 5, "sha256": hashlib.sha256(b"extra").hexdigest(), "mode": 0o600}
        self.manifest_edit(change)
        self.verify(ok=False)

    def test_database_tamper_rejected_even_with_new_file_hash(self):
        self.create()
        path = self.output / "auth/auth.sqlite3"
        with contextlib.closing(sqlite3.connect(path)) as db, db:
            db.execute("INSERT INTO recovery_pending VALUES(1)")
        def change(value):
            value["files"]["auth/auth.sqlite3"].update(size=path.stat().st_size, sha256=hashlib.sha256(path.read_bytes()).hexdigest())
        self.manifest_edit(change)
        self.verify(ok=False)

    def test_verify_detects_concurrent_change(self):
        self.create()
        function = MODULE["verify_tree"]
        original = function.__globals__["database"]
        calls = []
        def changed(conn, version):
            original(conn, version)
            if not calls:
                calls.append(True)
                self.write(self.output / "release/web/index.html", b"changed during verification")
        with mock.patch.dict(function.__globals__, database=changed):
            with self.assertRaises(ValueError):
                MODULE["verify"](str(self.output))

    def test_limits_are_checked_without_large_fixtures(self):
        function = MODULE["scan"]
        with mock.patch.dict(function.__globals__, MAX_FILE=8):
            with self.assertRaises(ValueError):
                MODULE["create"](self.args)
        self.assertTrue((self.output / "INCOMPLETE").exists())


if __name__ == "__main__":
    unittest.main()
