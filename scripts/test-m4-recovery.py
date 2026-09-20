#!/usr/bin/env python3
"""只使用虚构 SQLite/token 夹具，在 Linux 原生验证 M4 恢复包边界。"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import unittest

TOOL = Path(__file__).resolve().parents[1] / "deploy/agent/m4_recovery.py"


@unittest.skipUnless(sys.platform == "linux", "恢复文件操作器只在 Linux 部署")
class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="studiolo-m4-recovery-test-"))
        self.connections = []
        for name in ("data", "data/audit", "config", "bin", "policy"):
            (self.root / name).mkdir(mode=0o700)
        self.databases = {"data/agent.sqlite3": 0, "data/audit/audit.sqlite3": 2,
                          "policy/policy.sqlite3": 1}
        for name, version in self.databases.items():
            conn = sqlite3.connect(self.root / name)
            self.connections.append(conn)
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("PRAGMA wal_autocheckpoint=0")
            conn.execute("PRAGMA user_version=" + str(version))
            conn.execute("CREATE TABLE fixture(id INTEGER PRIMARY KEY, body TEXT)")
            conn.commit()
            conn.execute("INSERT INTO fixture VALUES(1, 'committed-in-wal')")
            conn.commit()
            for suffix in ("", "-wal", "-shm"):
                os.chmod(str(self.root / name) + suffix, 0o600)
        self.write("config/claude.token", b"fixture-only-not-a-real-token\n")
        self.write("bin/studiolo-agent", b"#!/bin/sh\nexit 0\n", 0o755)
        self.write("agent.service", b"[Unit]\nDescription=Recovery fixture\n", 0o644)

    def tearDown(self):
        for conn in self.connections:
            conn.close()
        shutil.rmtree(self.root)

    def write(self, name, raw, mode=0o600):
        path = self.root / name
        path.write_bytes(raw)
        path.chmod(mode)

    def call(self, *args, ok=True):
        result = subprocess.run([sys.executable, "-I", "-B", str(TOOL), *map(str, args)],
                                capture_output=True, text=True, timeout=20)
        self.assertEqual(result.returncode == 0, ok, result.stderr)
        self.assertNotIn("fixture-only-not-a-real-token", result.stdout + result.stderr)
        self.assertNotIn("committed-in-wal", result.stdout + result.stderr)
        return result

    def create(self, output="bundle", policy=False, ok=True, **overrides):
        options = {"data": self.root / "data", "config": self.root / "config",
                   "binary": self.root / "bin/studiolo-agent", "unit": self.root / "agent.service",
                   "output": self.root / output}
        if policy:
            options["policy"] = self.root / "policy" if policy is True else policy
        options.update(overrides)
        args = [part for key, value in options.items() for part in ("--" + key, value)]
        self.call("create", *args, ok=ok)
        return options["output"]

    def query(self, path):
        with sqlite3.connect("file:" + str(path) + "?mode=ro&immutable=1", uri=True) as conn:
            return conn.execute("SELECT body FROM fixture").fetchone()[0]

    def test_wal_snapshot_and_restore_to_new_directory(self):
        before = {name + suffix: hashlib.sha256((self.root / (name + suffix)).read_bytes()).hexdigest()
                  for name in self.databases for suffix in ("", "-wal")}
        bundle = self.create()
        self.call("verify", bundle)
        restored = self.root / "restored"
        self.call("restore-copy", bundle, restored)
        self.call("verify", restored)
        manifest = json.loads((bundle / "manifest.json").read_text())
        self.assertEqual(manifest["database_versions"], {"data/agent.sqlite3": 0, "data/audit/audit.sqlite3": 2})
        self.assertEqual(set(manifest["files"]), {"data/agent.sqlite3", "data/audit/audit.sqlite3",
                                                 "config/claude.token", "bin/studiolo-agent", "previous.service"})
        for root in (bundle, restored):
            for name in ("data/agent.sqlite3", "data/audit/audit.sqlite3"):
                self.assertEqual(self.query(root / name), "committed-in-wal")
            self.assertFalse((root / "INCOMPLETE").exists())
            for path in [root, *root.rglob("*")]:
                expected = 0o700 if path.is_dir() or path == root / "bin/studiolo-agent" else 0o600
                self.assertEqual(stat.S_IMODE(path.stat().st_mode), expected)
            self.assertEqual(subprocess.run([root / "bin/studiolo-agent"]).returncode, 0)
        for name, digest in before.items():
            self.assertEqual(hashlib.sha256((self.root / name).read_bytes()).hexdigest(), digest)

    def test_optional_policy_roundtrip(self):
        bundle = self.create(policy=True)
        self.call("verify", bundle)
        self.call("restore-copy", bundle, self.root / "restored")
        self.assertEqual(self.query(self.root / "restored/policy/policy.sqlite3"), "committed-in-wal")

    def test_closed_wal_databases_do_not_gain_source_sidecars(self):
        for conn in self.connections:
            conn.close()
        self.connections.clear()
        before = set(self.root.rglob("*-wal")) | set(self.root.rglob("*-shm"))
        self.assertEqual(before, set())
        bundle = self.create(policy=True)
        self.call("verify", bundle)
        self.assertEqual(set(self.root.rglob("*-wal")) | set(self.root.rglob("*-shm")), set())

    def test_missing_policy_database_leaves_incomplete_bundle(self):
        empty = self.root / "empty-policy"
        empty.mkdir(mode=0o700)
        bundle = self.create(policy=empty, ok=False)
        self.assertTrue((bundle / "INCOMPLETE").exists())
        self.call("verify", bundle, ok=False)

    def test_unknown_database_versions_rejected(self):
        for index, conn in enumerate(self.connections):
            with self.subTest(database=index):
                conn.execute("PRAGMA user_version=99")
                bundle = self.create(output="bad-version-" + str(index), policy=True, ok=False)
                self.assertTrue((bundle / "INCOMPLETE").exists())
                self.call("verify", bundle, ok=False)
                conn.execute("PRAGMA user_version=" + str(list(self.databases.values())[index]))

    def test_create_and_restore_never_replace_existing_output(self):
        existing = self.root / "existing"
        existing.mkdir(mode=0o700)
        self.write("existing/keep", b"keep")
        self.create(output="existing", ok=False)
        bundle = self.create()
        self.call("restore-copy", bundle, existing, ok=False)
        self.assertEqual(list(existing.iterdir()), [existing / "keep"])
        self.assertEqual((existing / "keep").read_bytes(), b"keep")

    def test_symlink_ancestor_and_parent_traversal_rejected(self):
        alias = self.root / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        self.create(output="bad-source", data=alias / "data", ok=False)
        self.create(output="alias/new-output", ok=False)
        self.create(output="missing/../escape", ok=False)
        self.assertFalse((self.root / "new-output").exists())
        self.assertFalse((self.root / "escape").exists())

    def test_symlink_source_file_rejected(self):
        for index, name in enumerate(("config/claude.token", "bin/studiolo-agent", "agent.service")):
            with self.subTest(source=name):
                original = self.root / name
                moved = self.root / ("saved-source-" + str(index))
                original.rename(moved)
                original.symlink_to(moved)
                self.create(output="bad-symlink-" + str(index), ok=False)
                original.unlink()
                moved.rename(original)

    def test_hardlinked_sources_rejected(self):
        for index, name in enumerate(("data/agent.sqlite3", "data/audit/audit.sqlite3",
                                     "config/claude.token", "bin/studiolo-agent", "agent.service")):
            with self.subTest(source=name):
                extra = self.root / "extra-link"
                os.link(self.root / name, extra)
                self.create(output="bad-hardlink-" + str(index), ok=False)
                extra.unlink()

    def test_source_permissions_and_unknown_config_files_rejected(self):
        for index, name in enumerate(("data", "config", "config/claude.token", "bin/studiolo-agent")):
            with self.subTest(source=name):
                path = self.root / name
                mode = stat.S_IMODE(path.stat().st_mode)
                path.chmod(mode | 0o020)
                self.create(output="bad-mode-" + str(index), ok=False)
                path.chmod(mode)
        self.write("config/unexpected.json", b"{}")
        self.create(output="bad-config", ok=False)

    def test_bundle_tampering_missing_and_extra_files_rejected(self):
        bundle = self.create()
        token = bundle / "config/claude.token"
        original = token.read_bytes()
        token.write_bytes(b"tampered")
        self.call("verify", bundle, ok=False)
        self.call("restore-copy", bundle, self.root / "tampered-restore", ok=False)
        self.assertFalse((self.root / "tampered-restore").exists())
        token.unlink()
        self.call("verify", bundle, ok=False)
        token.write_bytes(original)
        token.chmod(0o600)
        extra = bundle / "extra"
        extra.write_bytes(b"extra")
        extra.chmod(0o600)
        self.call("verify", bundle, ok=False)
        extra.unlink()
        (bundle / "unknown-dir").mkdir(mode=0o700)
        self.call("verify", bundle, ok=False)

    def test_bundle_links_and_permissions_rejected(self):
        bundle = self.create()
        token = bundle / "config/claude.token"
        os.link(token, self.root / "linked-token")
        self.call("verify", bundle, ok=False)
        (self.root / "linked-token").unlink()
        token.chmod(0o644)
        self.call("verify", bundle, ok=False)
        token.chmod(0o600)
        (bundle / "bin/studiolo-agent").chmod(0o600)
        self.call("verify", bundle, ok=False)

    def test_manifest_shape_and_incomplete_marker_rejected(self):
        bundle = self.create()
        path = bundle / "manifest.json"
        original = path.read_bytes()
        for raw in (b'{"schema_version":1,"schema_version":1}',
                    original.replace(b'"schema_version": 1', b'"schema_version": true'),
                    original.replace(b'"size": ', b'"unexpected": ')):
            path.write_bytes(raw)
            self.call("verify", bundle, ok=False)
        path.write_bytes(original)
        (bundle / "INCOMPLETE").write_bytes(b"incomplete\n")
        self.call("verify", bundle, ok=False)

    def test_output_cannot_be_inside_source(self):
        self.create(output="data/nested-output", ok=False)
        self.assertFalse((self.root / "data/nested-output").exists())
        bundle = self.create()
        self.call("restore-copy", bundle, bundle / "nested-output", ok=False)
        self.assertFalse((bundle / "nested-output").exists())
        self.call("verify", bundle)

    def test_foreign_key_violation_rejected(self):
        conn = self.connections[0]
        conn.execute("CREATE TABLE parent(id INTEGER PRIMARY KEY)")
        conn.execute("CREATE TABLE child(parent_id INTEGER REFERENCES parent(id))")
        conn.execute("INSERT INTO child VALUES(999)")
        conn.commit()
        bundle = self.create(ok=False)
        self.assertTrue((bundle / "INCOMPLETE").exists())


if __name__ == "__main__":
    unittest.main()
