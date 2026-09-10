#!/usr/bin/env python3
"""客户端上下文存储边界：使用真实 schema、私有目录与故障注入。"""
import copy
import fcntl
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
import client_projection_store as store
from client_projection_render import render
from client_projection_schema import make_bundle


def bundle(body="规则原文", client="claude", timestamp=1000):
    cfg = {"schema_version": 1, "client": client, "project": "项目/测试",
           "source_url": "https://example.com/read", "token_env": "STUDIOLO_TOKEN",
           "rules": ["_规则/通用工作规则.md"], "skills": []}
    snapshot = {"identity": {"type": client, "instance": "0" * 26},
                "freshness": {"generation": "a" * 32, "source_digest": "b" * 64,
                              "source_observed_at_ms": 1000, "received_at_ms": timestamp},
                "generated_at_ms": timestamp,
                "sources": [{"path": cfg["rules"][0], "revision": "c" * 64, "body": body}]}
    return make_bundle(cfg, snapshot, render(cfg, snapshot))


class ProjectionStoreTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(dir="/private/tmp" if sys.platform == "darwin" else None)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.target = self.root / "context"
        self.first = bundle()

    def current(self):
        return self.target / os.readlink(self.target / "current")

    def install(self):
        return store.apply(self.target, self.first)

    def test_plan_and_check_do_not_create_missing_target_or_lock(self):
        self.assertEqual(store.check(self.target)["status"], "empty")
        result = store.plan(self.target, self.first)
        self.assertEqual(result["status"], "create")
        self.assertFalse(self.target.exists())
        self.assertNotIn("规则原文", json.dumps(result, ensure_ascii=False))

    def test_empty_private_target_is_not_modified_by_plan(self):
        self.target.mkdir(mode=0o700)
        self.assertEqual(store.plan(self.target, self.first)["status"], "create")
        self.assertEqual(list(self.target.iterdir()), [])

    def test_missing_multiple_parent_directories_are_read_only_empty(self):
        target = self.root / "missing" / "nested" / "context"
        self.assertEqual(store.check(target)["status"], "empty")
        self.assertEqual(store.plan(target, self.first)["status"], "create")
        self.assertEqual(list(self.root.iterdir()), [])

    def test_both_native_clients_keep_hidden_files_in_complete_generation(self):
        for client in ("claude", "codex"):
            with self.subTest(client=client):
                payload = bundle(client=client)
                target = self.root / client
                self.assertEqual(store.apply(target, payload)["status"], "create")
                actual = store.check(target)
                self.assertEqual(actual["status"], "ok")
                self.assertEqual(actual["manifest"], payload["manifest"])
                current = target / os.readlink(target / "current")
                for path, raw in payload["files"].items():
                    self.assertEqual((current / path).read_bytes(), raw)
                    self.assertEqual((current / path).stat().st_mode & 0o777, 0o600)
                self.assertEqual(target.stat().st_mode & 0o777, 0o700)

    def test_same_projection_preserves_original_receipt_and_pointer(self):
        self.install()
        previous = os.lstat(self.target / "current")
        later = bundle(timestamp=2000)
        self.assertNotEqual(later["manifest"]["bundle_id"], self.first["manifest"]["bundle_id"])
        self.assertEqual(store.plan(self.target, later)["status"], "unchanged")
        self.assertEqual(store.apply(self.target, later)["bundle_id"], self.first["manifest"]["bundle_id"])
        self.assertEqual(os.lstat(self.target / "current").st_ino, previous.st_ino)
        self.assertEqual(len(list((self.target / "releases").iterdir())), 1)

    def test_changed_source_replaces_pointer_and_preserves_old_generation(self):
        self.install()
        previous = self.current()
        second = bundle("更新规则")
        planned = store.plan(self.target, second)
        self.assertEqual(planned["status"], "changed")
        self.assertTrue(any(f["old_sha256"] != f["new_sha256"] for f in planned["files"]))
        self.assertEqual(store.apply(self.target, second)["status"], "changed")
        self.assertNotEqual(self.current(), previous)
        self.assertEqual((previous / "sources/_规则/通用工作规则.md").read_text(), "规则原文")

    def test_existing_user_directory_and_permissions_are_never_adopted(self):
        self.target.mkdir(mode=0o700)
        user = self.target / "CLAUDE.md"
        user.write_text("用户配置")
        self.assertEqual(store.plan(self.target, self.first)["status"], "conflict")
        with self.assertRaises(ValueError):
            self.install()
        self.assertEqual(user.read_text(), "用户配置")
        self.assertEqual({p.name for p in self.target.iterdir()}, {"CLAUDE.md"})
        user.unlink()
        self.target.chmod(0o755)
        with self.assertRaises(ValueError):
            self.install()
        self.assertEqual(list(self.target.iterdir()), [])

    def test_changed_missing_and_extra_current_files_block_application(self):
        self.install()
        original = self.current() / "CLAUDE.md"
        saved = original.read_bytes()
        previous = os.readlink(self.target / "current")
        mutations = [lambda: original.write_text("手工修改"), lambda: original.unlink(),
                     lambda: (self.current() / "user.txt").write_text("用户新增")]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                mutation()
                self.assertEqual(store.check(self.target)["status"], "local_modified")
                self.assertEqual(store.plan(self.target, bundle("新规则"))["status"], "conflict")
                with self.assertRaises((OSError, ValueError)):
                    store.apply(self.target, bundle("新规则"))
                self.assertEqual(os.readlink(self.target / "current"), previous)
                original.write_bytes(saved)
                original.chmod(0o600)
                extra = self.current() / "user.txt"
                if extra.exists():
                    extra.unlink()

    def test_manifest_metadata_tampering_is_detected(self):
        self.install()
        path = self.current() / "manifest.json"
        value = json.loads(path.read_text())
        value["generated_at_ms"] += 1
        path.write_text(json.dumps(value))
        self.assertEqual(store.check(self.target)["status"], "local_modified")
        with self.assertRaises(ValueError):
            store.apply(self.target, bundle("更新"))

    def test_current_link_cannot_escape_store(self):
        self.install()
        link = self.target / "current"
        link.unlink()
        link.symlink_to(self.root)
        self.assertEqual(store.check(self.target)["status"], "local_modified")
        with self.assertRaises(ValueError):
            store.apply(self.target, bundle("更新"))
        self.assertEqual(os.readlink(link), str(self.root))

    def test_deleted_current_is_drift_and_cannot_be_recreated_by_apply(self):
        self.install()
        previous = self.current()
        (self.target / "current").unlink()
        names = sorted(p.name for p in self.target.iterdir())
        releases = sorted(p.name for p in (self.target / "releases").iterdir())
        second = bundle("不应应用的新规则")
        self.assertEqual(store.check(self.target)["status"], "local_modified")
        self.assertEqual(store.plan(self.target, second)["status"], "conflict")
        with self.assertRaises(ValueError):
            store.apply(self.target, second)
        self.assertFalse(os.path.lexists(self.target / "current"))
        self.assertEqual(sorted(p.name for p in self.target.iterdir()), names)
        self.assertEqual(sorted(p.name for p in (self.target / "releases").iterdir()), releases)
        self.assertEqual((previous / "sources/_规则/通用工作规则.md").read_text(), "规则原文")

    def test_parent_symlink_is_rejected_without_touching_destination(self):
        alias = self.root / "alias"
        alias.symlink_to(self.root)
        self.assertEqual(store.plan(alias / "context", self.first)["status"], "conflict")
        with self.assertRaises((OSError, ValueError)):
            store.apply(alias / "context", self.first)
        self.assertFalse(self.target.exists())

    def test_lock_excludes_another_apply_without_partial_release(self):
        self.install()
        with (self.target / "projection.lock").open("rb") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                store.apply(self.target, bundle("更新"))
        self.assertEqual(len(list((self.target / "releases").iterdir())), 1)

    def test_partial_write_failure_keeps_previous_and_preserves_partial_files(self):
        self.install()
        previous = os.readlink(self.target / "current")
        second = bundle("更新")
        original, calls = store.write_path, []

        def fail(root, path, raw):
            calls.append(path)
            if len(calls) == 2:
                raise OSError("模拟中断")
            original(root, path, raw)

        with patch.object(store, "write_path", fail), self.assertRaises(OSError):
            store.apply(self.target, second)
        self.assertEqual(os.readlink(self.target / "current"), previous)
        partial = self.target / "releases" / second["manifest"]["bundle_id"]
        self.assertTrue((partial / calls[0]).exists())
        self.assertEqual(store.check(self.target)["status"], "ok")
        with self.assertRaises((OSError, ValueError)):
            store.apply(self.target, second)

    def test_user_edit_during_staging_is_kept_and_blocks_activation(self):
        self.install()
        old = self.current() / "CLAUDE.md"
        previous = os.readlink(self.target / "current")
        original = store._activate

        def edit_then_activate(*args):
            old.write_text("并发用户编辑")
            return original(*args)

        with patch.object(store, "_activate", edit_then_activate), self.assertRaises(ValueError):
            store.apply(self.target, bundle("更新"))
        self.assertEqual(os.readlink(self.target / "current"), previous)
        self.assertEqual(old.read_text(), "并发用户编辑")

    def test_root_rebinding_during_staging_rejects_without_touching_new_root(self):
        self.install()
        moved = self.root / "moved"
        original = store._activate

        def move_then_activate(*args):
            self.target.rename(moved)
            self.target.mkdir(mode=0o700)
            return original(*args)

        with patch.object(store, "_activate", move_then_activate), self.assertRaises(ValueError):
            store.apply(self.target, bundle("更新"))
        self.assertEqual(list(self.target.iterdir()), [])
        self.assertEqual(os.readlink(moved / "current"), "releases/" + self.first["manifest"]["bundle_id"])

    def test_corrupt_candidate_is_rejected_before_target_creation(self):
        candidate = copy.deepcopy(self.first)
        candidate["files"]["CLAUDE.md"] = b"tampered"
        with self.assertRaises(ValueError):
            store.apply(self.target, candidate)
        self.assertFalse(self.target.exists())

    def test_fsync_failure_after_pointer_commit_reports_actual_new_generation(self):
        self.install()
        previous = self.current()
        second = bundle("更新后持久化待确认")
        next_link = "releases/" + second["manifest"]["bundle_id"]
        original = os.fsync

        def fail_after_commit(fd):
            if os.readlink(self.target / "current") == next_link:
                raise OSError("模拟发布后磁盘同步失败")
            return original(fd)

        with patch.object(os, "fsync", fail_after_commit):
            result = store.apply(self.target, second)
        self.assertEqual(result["status"], "activated_durability_unknown")
        self.assertEqual(result["current_bundle_id"], second["manifest"]["bundle_id"])
        self.assertEqual(os.readlink(self.target / "current"), next_link)
        self.assertTrue(previous.is_dir())
        self.assertEqual(store.check(self.target)["status"], "ok")


if __name__ == "__main__":
    unittest.main()
