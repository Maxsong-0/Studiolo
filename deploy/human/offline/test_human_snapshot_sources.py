#!/usr/bin/env python3
"""仅原生 Linux 的虚构来源边界测试；通过 unittest discover 运行整组。"""
import hashlib
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
    def test_only_manifested_example_vault_gitkeep_leaf_is_allowed(self):
        self.create()
        self.verify()
        self.assertTrue((self.output / "release/example-vault/附件演示/空目录/.gitkeep").is_file())
        for path in (".gitkeep", "web/.gitkeep", "example-vault/.hidden", "example-vault/.dir/.gitkeep",
                     "example-vault/../.gitkeep", "example-vault/./.gitkeep"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                MODULE["safe"](path)

    def test_writable_and_symlink_ancestors(self):
        ancestor = self.root / "unsafe"
        ancestor.mkdir(mode=0o700)
        self.auth.rename(ancestor / "auth")
        self.args.auth_dir = str(ancestor / "auth")
        ancestor.chmod(0o777)
        self.rejected_incomplete()
        shutil.rmtree(self.output)
        ancestor.chmod(0o700)
        alias = self.root / "alias"
        alias.symlink_to(ancestor, target_is_directory=True)
        self.args.auth_dir = str(alias / "auth")
        self.rejected_incomplete()

    def test_source_symlink_hardlink_and_fifo(self):
        extra = self.release / "unlisted"
        for kind in ("symlink", "hardlink", "fifo"):
            with self.subTest(kind=kind):
                if kind == "symlink":
                    extra.symlink_to(self.config)
                elif kind == "hardlink":
                    os.link(self.config, extra)
                else:
                    os.mkfifo(extra, 0o600)
                self.rejected_incomplete()
                extra.unlink()
                shutil.rmtree(self.output)

    def test_cross_mount_and_device_rejected_without_mounting(self):
        with __import__("contextlib").ExitStack() as stack:
            tree = MODULE["Tree"](str(self.auth), stack)
            with tree.file("auth.sqlite3") as fd:
                globals_ = tree.node.__func__.__globals__
                with mock.patch.dict(globals_, mount_id=lambda _: tree.mount + 1):
                    with self.assertRaises(ValueError):
                        tree.node(fd, False)
                tree.device += 1
                with self.assertRaises(ValueError):
                    tree.node(fd, False)
                tree.device -= 1

    def test_extra_source_database_files_rejected(self):
        for name in ("unlisted", "auth.sqlite3-journal", "auth.sqlite3-wal"):
            with self.subTest(name=name):
                self.write(self.auth / name, b"fixture")
                self.rejected_incomplete()
                (self.auth / name).unlink()
                shutil.rmtree(self.output)

    def test_release_mismatch_missing_extra_and_duplicate_keys(self):
        original = self.json(self.release_manifest)
        manifest = self.release / "manifest.json"
        broken = json.loads(original)
        broken["files"]["web/index.html"]["bytes"] += 1
        for raw in (self.json(broken), original.replace(b'"build":', b'"build":"duplicate", "build":'),
                    self.json(dict(self.release_manifest, unexpected=1))):
            self.write(manifest, raw)
            self.rejected_incomplete()
            shutil.rmtree(self.output)
        self.write(manifest, original)
        self.write(self.release / "extra", b"unexpected")
        self.rejected_incomplete()
        (self.release / "extra").unlink()
        shutil.rmtree(self.output)
        (self.release / "web/index.html").unlink()
        self.rejected_incomplete()

    def test_manifest_dangerous_and_non_nfc_paths_rejected(self):
        for name in ("../escape", "/absolute", "web/../escape", "web\\escape", "web/e\u0301.html", "web/new\nline"):
            with self.subTest(name=name):
                value = dict(self.release_manifest)
                value["files"] = dict(value["files"], **{name: {"bytes": 1, "sha256": "0" * 64}})
                self.write(self.release / "manifest.json", self.json(value))
                self.rejected_incomplete()
                shutil.rmtree(self.output)

    def test_existing_target_and_symlink_never_overwritten(self):
        self.output.mkdir(mode=0o700)
        self.write(self.output / "sentinel", b"keep")
        self.create(ok=False)
        self.assertEqual(set(p.name for p in self.output.iterdir()), {"sentinel"})
        self.assertEqual((self.output / "sentinel").read_bytes(), b"keep")
        shutil.rmtree(self.output)
        self.output.symlink_to(self.auth, target_is_directory=True)
        before = self.sources()
        self.create(ok=False)
        self.assertEqual(before, self.sources())

    def test_nested_output_rejected(self):
        self.args.output = str(self.auth / "nested")
        self.create(ok=False)
        self.assertFalse((self.auth / "nested").exists())

    def test_source_content_inode_and_new_entry_changes_leave_incomplete(self):
        path = self.release / "web/index.html"
        def replace():
            raw = path.read_bytes()
            path.unlink()
            self.write(path, raw)
        changes = [lambda: self.write(path, b"changed during copy"), replace,
                   lambda: self.write(self.auth / "late", b"late")]
        for change in changes:
            with self.subTest(change=change.__name__):
                saved = path.read_bytes()
                with self.assertRaises(ValueError):
                    MODULE["create"](self.args, between_scans=change)
                self.assertTrue((self.output / "INCOMPLETE").is_file())
                self.verify(ok=False)
                shutil.rmtree(self.output)
                self.write(path, saved, 0o644)
                (self.auth / "late").unlink(missing_ok=True)



if __name__ == "__main__":
    unittest.main()
