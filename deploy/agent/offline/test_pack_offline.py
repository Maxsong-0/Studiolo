#!/usr/bin/env python3
"""仅虚构 SQLite、token 与 replica；由主线在 107 原生 Linux 执行。"""
import argparse
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
import types
import unittest

TOOL = Path(__file__).with_name("pack_offline.py")
SOURCE = None


@unittest.skipUnless(sys.platform == "linux", "只允许原生 Linux")
class OfflineTests(unittest.TestCase):
    def setUp(self):
        self.old_umask = os.umask(0o077)
        self.root = Path(tempfile.mkdtemp(prefix="studiolo-offline-fixture-"))
        self.tools = self.root / "tools"
        self.tools.mkdir(mode=0o700)
        for name in ("m4_recovery.py", "replica_common.py", "replica_store.py", "replica_cleanup.py"):
            origin = SOURCE / ("deploy/agent" if name == "m4_recovery.py" else "scripts") / name
            self.write(self.tools / name, origin.read_bytes())
        self.write(self.tools / "fixture-probe.py", b"# fixture probe\n")
        source = self.root / "source"
        for directory in (source, source / "data", source / "data/audit", source / "config", source / "policy"):
            directory.mkdir(mode=0o700)
        for name, version in (("data/agent.sqlite3", 0), ("data/audit/audit.sqlite3", 2), ("policy/policy.sqlite3", 1)):
            with closing(sqlite3.connect(source / name)) as conn, conn:
                conn.execute("CREATE TABLE fixture(value TEXT)")
                conn.execute("INSERT INTO fixture VALUES ('offline-fictional-record')")
                conn.execute("PRAGMA user_version=" + str(version))
            (source / name).chmod(0o600)
        self.write(source / "config/claude.token", b"offline-fictional-token\n")
        self.write(source / "binary", b"#!/bin/sh\nexit 0\n", 0o700)
        self.write(source / "unit", b"[Service]\nExecStart=/fixture/binary\n")
        self.recovery = self.root / "recovery"
        self.run_command(self.tools / "m4_recovery.py", "create", "--data", source / "data",
                         "--config", source / "config", "--binary", source / "binary", "--unit", source / "unit",
                         "--policy", source / "policy", "--output", self.recovery)
        self.replica = self.root / "replica"
        for directory in (self.replica, self.replica / "control", self.replica / "releases"):
            directory.mkdir(mode=0o700)
        self.write(self.replica / "sync.lock", b"")
        raw = b"fictional note\n"
        manifest = {"schema_version": 1, "allow": ["_规则", "项目/Studiolo", "项目/Tillerwright"], "entries": [
            {"path": "项目/Studiolo/note.md", "kind": "file", "bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}]}
        encoded = json.dumps(manifest, ensure_ascii=False).encode()
        for generation in ("a" * 32, "b" * 32):
            release = self.replica / "releases" / generation
            for suffix in ("", "vault", "vault/_规则", "vault/项目", "vault/项目/Studiolo"):
                (release / suffix).mkdir(mode=0o700)
            self.write(release / "manifest.json", encoded)
            self.write(release / "vault/项目/Studiolo/note.md", raw)
        now = time.time_ns() // 1_000_000
        state = {"schema_version": 1, "status": "ready", "last_attempt_at_ms": now, "last_error": None,
                 "receipt": {"generation": "a" * 32, "source_digest": hashlib.sha256(encoded).hexdigest(),
                             "source_observed_at_ms": now - 86_400_000, "received_at_ms": now - 86_400_000}}
        self.write(self.replica / "control/state.json", json.dumps(state).encode())
        self.write(self.root / "runtime.txt", b"fixture: no real dynamic dependencies\n")
        self.write(self.root / "release.txt", b"ID=fictional-linux\n")
        self.output = self.root / "offline"
        self.args = types.SimpleNamespace(recovery=str(self.recovery), replica=str(self.replica), tools=str(self.tools),
            runtime_libraries=str(self.root / "runtime.txt"), os_release=str(self.root / "release.txt"), output=str(self.output))

    def tearDown(self):
        shutil.rmtree(self.root)
        os.umask(self.old_umask)

    def write(self, path, raw, permissions=0o600):
        path.write_bytes(raw)
        path.chmod(permissions)

    def run_command(self, tool, *args, ok=True):
        result = subprocess.run([sys.executable, "-I", "-B", str(tool), *map(str, args)],
                                capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode == 0, ok, result.stderr)
        self.assertNotIn("offline-fictional-token", result.stdout + result.stderr)
        self.assertNotIn("offline-fictional-record", result.stdout + result.stderr)
        return result

    def pack(self, ok=True):
        flags = [part for key, value in vars(self.args).items() for part in ("--" + key.replace("_", "-"), value)]
        return self.run_command(TOOL, "pack", *flags, ok=ok)

    def verify(self, ok=True):
        return self.run_command(TOOL, "verify", self.output, "--tools", self.tools, ok=ok)

    def snapshot(self):
        roots = [self.recovery, self.replica, self.tools, self.root / "runtime.txt", self.root / "release.txt"]
        paths = [path for root in roots for path in ([root, *root.rglob("*")] if root.is_dir() else [root])]
        return {str(path): (path.stat().st_ino, path.stat().st_mode, path.stat().st_uid,
                path.stat().st_mtime_ns, path.stat().st_ctime_ns, None if path.is_dir() else path.read_bytes()) for path in paths}

    def test_roundtrip_all_generations_modes_and_unchanged_sources(self):
        before = self.snapshot()
        state = (self.replica / "control/state.json").read_bytes()
        self.pack()
        self.verify()
        self.assertEqual(before, self.snapshot())
        self.assertEqual(state, (self.output / "replica/control/state.json").read_bytes())
        self.assertEqual({p.name for p in (self.output / "replica/releases").iterdir()}, {"a" * 32, "b" * 32})
        self.run_command(self.tools / "m4_recovery.py", "verify", self.output / "recovery")
        manifest = json.loads((self.output / "manifest.json").read_bytes())
        self.assertFalse((self.output / "INCOMPLETE").exists())
        for path in [self.output, *self.output.rglob("*")]:
            expected = 0o700 if path.is_dir() or path == self.output / "recovery/bin/studiolo-agent" else 0o600
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), expected)
        self.assertEqual(manifest["files"]["recovery/bin/studiolo-agent"]["mode"], 0o700)

    def test_existing_output_is_not_overwritten(self):
        self.output.mkdir(mode=0o700)
        self.write(self.output / "sentinel", b"keep")
        before = self.snapshot()
        self.pack(ok=False)
        self.assertEqual((self.output / "sentinel").read_bytes(), b"keep")
        self.assertEqual(set(p.name for p in self.output.iterdir()), {"sentinel"})
        self.assertEqual(before, self.snapshot())

    def test_source_symlink_is_rejected(self):
        (self.tools / "alias.py").symlink_to(self.tools / "fixture-probe.py")
        self.pack(ok=False)

    def test_source_hardlink_is_rejected(self):
        os.link(self.tools / "fixture-probe.py", self.tools / "hardlink.py")
        self.pack(ok=False)

    def test_source_special_file_is_rejected(self):
        os.mkfifo(self.tools / "fifo", 0o600)
        self.pack(ok=False)

    def test_symlink_ancestor_is_rejected(self):
        alias = self.root / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        self.args.replica = str(alias / "replica")
        self.pack(ok=False)

    def test_output_symlink_is_rejected(self):
        self.output.symlink_to(self.recovery, target_is_directory=True)
        before = self.snapshot()
        self.pack(ok=False)
        self.assertEqual(before, self.snapshot())

    def test_source_private_mode_is_required(self):
        (self.replica / "control/state.json").chmod(0o644)
        self.pack(ok=False)

    def test_source_change_leaves_incomplete_bundle(self):
        module = runpy.run_path(str(TOOL))
        def mutate():
            self.write(self.tools / "fixture-probe.py", b"# changed during copy\n")
        with self.assertRaises(ValueError):
            module["pack"](self.args, between_scans=mutate)
        self.assertTrue((self.output / "INCOMPLETE").is_file())
        self.verify(ok=False)

    def test_source_new_entry_is_detected(self):
        module = runpy.run_path(str(TOOL))
        with self.assertRaises(ValueError):
            module["pack"](self.args, between_scans=lambda: self.write(self.tools / "late.py", b"# late\n"))
        self.assertTrue((self.output / "INCOMPLETE").is_file())

    def test_corrupt_output_is_detected(self):
        self.pack()
        self.write(self.output / "system/os-release", b"corrupted")
        self.verify(ok=False)

    def test_missing_output_is_detected(self):
        self.pack()
        (self.output / "system/os-release").unlink()
        self.verify(ok=False)

    def test_extra_file_is_detected(self):
        self.pack()
        self.write(self.output / "tools/extra.py", b"extra")
        self.verify(ok=False)

    def test_extra_empty_directory_is_detected(self):
        self.pack()
        (self.output / "tools/extra").mkdir(mode=0o700)
        self.verify(ok=False)

    def test_output_mode_change_is_detected(self):
        self.pack()
        (self.output / "recovery/bin/studiolo-agent").chmod(0o600)
        self.verify(ok=False)

    def test_duplicate_manifest_keys_are_rejected(self):
        self.pack()
        path = self.output / "manifest.json"
        raw = path.read_bytes().replace(b'"schema_version": 1', b'"schema_version": 1, "schema_version": 1')
        self.write(path, raw)
        self.verify(ok=False)

    def test_replica_partial_stage_is_rejected(self):
        (self.replica / (".stage-" + "c" * 32)).mkdir(mode=0o700)
        self.pack(ok=False)

    def test_replica_invalid_receipt_is_rejected_and_output_retained(self):
        path = self.replica / "control/state.json"
        value = json.loads(path.read_bytes())
        value["receipt"]["source_digest"] = "0" * 64
        self.write(path, json.dumps(value).encode())
        before = self.snapshot()
        self.pack(ok=False)
        self.assertTrue((self.output / "INCOMPLETE").is_file())
        self.assertEqual(before, self.snapshot())

    def test_invalid_inner_recovery_is_rejected(self):
        self.write(self.recovery / "config/claude.token", b"changed-fixture")
        self.pack(ok=False)

    def test_required_tool_is_mandatory(self):
        (self.tools / "replica_common.py").unlink()
        self.pack(ok=False)

    def test_system_inputs_can_have_same_basename(self):
        for number in (1, 2):
            directory = self.root / str(number)
            directory.mkdir(mode=0o700)
            self.write(directory / "same", str(number).encode())
        self.args.runtime_libraries = str(self.root / "1/same")
        self.args.os_release = str(self.root / "2/same")
        self.pack()
        self.verify()
        self.assertEqual((self.output / "system/runtime-libraries.txt").read_bytes(), b"1")
        self.assertEqual((self.output / "system/os-release").read_bytes(), b"2")

    def test_unknown_top_level_file_even_if_manifest_updated_is_rejected(self):
        self.pack()
        self.write(self.output / "unexpected", b"extra")
        path = self.output / "manifest.json"
        value = json.loads(path.read_bytes())
        value["files"]["unexpected"] = {"sha256": hashlib.sha256(b"extra").hexdigest(), "size": 5, "mode": 0o600}
        self.write(path, json.dumps(value).encode())
        self.verify(ok=False)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True, help="只读取实际恢复和 replica 验证源码")
    args, remaining = parser.parse_known_args()
    SOURCE = args.source
    unittest.main(argv=[sys.argv[0], *remaining])
