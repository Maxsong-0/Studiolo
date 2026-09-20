#!/usr/bin/env python3
"""有界 Linux 虚构夹具；不载入生产配置或凭据。"""
import hashlib
import contextlib
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
import types
import unittest
from unittest import mock

TOOL = Path(__file__).with_name("human_snapshot.py")
runpy.run_path(str(TOOL))
MODULE = {}
for module_name in ("human_snapshot_io", "human_snapshot_format", "human_snapshot_core"):
    MODULE.update(vars(sys.modules[module_name]))
FIXTURE = "fictional-private-record-not-for-output"


@unittest.skipUnless(sys.platform == "linux", "仅原生 Linux")
class Fixture(unittest.TestCase):
    def setUp(self):
        self.old_umask = os.umask(0o077)
        self.root = Path(tempfile.mkdtemp(prefix="studiolo-human-snapshot-fixture-"))
        self.addCleanup(os.umask, self.old_umask)
        self.addCleanup(shutil.rmtree, self.root)
        self.auth, self.audit, self.release = (self.root / p for p in ("auth-source", "audit-source", "release-source"))
        for directory in (self.auth, self.audit, self.release, self.root / ".config"):
            directory.mkdir(mode=0o700)
        self.config = self.root / ".config/owner.json"
        self.portable = self.root / "portable.age"
        self.output = self.root / "human"
        with contextlib.closing(sqlite3.connect(self.auth / "auth.sqlite3")) as db, db:
            db.executescript("""
                PRAGMA user_version=4;
                CREATE TABLE auth_meta(id INTEGER PRIMARY KEY, store_id TEXT, epoch INTEGER, clock_floor INTEGER);
                INSERT INTO auth_meta VALUES(1,'0123456789abcdef0123456789abcdef',8,1700000000000);
                CREATE TABLE totp_factor(id INTEGER PRIMARY KEY, seed_ref TEXT, seed_digest TEXT, last_step INTEGER,
                    consumption_json TEXT, enrollment_json TEXT, intent_receipt TEXT, outcome_receipt TEXT, confirmed_at INTEGER);
                INSERT INTO totp_factor VALUES(1,'studiolo-human-totp-v2','fictional-digest',99,
                    'fictional-consumption','fictional-enrollment','fictional-intent','fictional-outcome',5);
                CREATE TABLE totp_pending(id INTEGER PRIMARY KEY);
                CREATE TABLE recovery_pending(id INTEGER PRIMARY KEY);
                CREATE TABLE recovery_codes(slot INTEGER PRIMARY KEY, digest TEXT);
                INSERT INTO recovery_codes VALUES(0,'fictional-code-digest');
                CREATE TABLE sessions(id INTEGER PRIMARY KEY, marker TEXT);
                INSERT INTO sessions VALUES(1,'fictional-session');
            """)
            db.execute("INSERT INTO sessions VALUES(2,?)", (FIXTURE,))
        with contextlib.closing(sqlite3.connect(self.audit / "audit.sqlite3")) as db, db:
            db.executescript("PRAGMA user_version=2; CREATE TABLE records(id INTEGER PRIMARY KEY, record TEXT);")
            db.execute("INSERT INTO records VALUES(1,?)", (FIXTURE,))
        self.document = {"version": 1, "auth_directory": str(self.auth), "audit_directory": str(self.audit),
                         "owner": {"issuer": "https://fictional.invalid", "audience": "fixture-audience",
                                   "subject": "fixture-subject", "email": "fixture@example.invalid"},
                         "idle_seconds": 1800, "absolute_seconds": 43200, "seed_ref": "studiolo-human-totp-v2"}
        self.write(self.config, self.json(self.document))
        self.write(self.portable, b"age-encryption.org/v1\n" + b"fictional-encrypted-payload" * 4)
        for path in ("bin", "web", "web/assets", "example-vault", "example-vault/附件演示", "example-vault/附件演示/空目录"):
            (self.release / path).mkdir(mode=0o700)
        payload = {"bin/studiolo-local-server": (b"fictional-native-product", 0o755),
                   "bin/studiolo-auth-admin": (b"fictional-admin", 0o700),
                   "launch-human.py": (b"raise RuntimeError('must never execute packaged code')\n", 0o600),
                   "studiolo-human.service": (b"[Service]\nExecStart=/fictional/product\n", 0o600),
                   "web/index.html": (b"<html>fictional</html>", 0o644),
                   "web/assets/app.js": (b"/* full release fixture */", 0o644),
                   "example-vault/note.md": (b"fictional example vault", 0o600),
                   "example-vault/附件演示/空目录/.gitkeep": (b"", 0o600),
                   "runtime-libraries.txt": (b"fictional-dependency", 0o600)}
        for name, (raw, mode) in payload.items():
            self.write(self.release / name, raw, mode)
        self.release_manifest = {"build": "fixture-build", "source_commit": "f" * 40, "assembly": "fixture-r7", "files": {
            name: {"bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()} for name, (raw, _) in payload.items()}}
        self.write(self.release / "manifest.json", self.json(self.release_manifest))
        self.args = types.SimpleNamespace(auth_dir=str(self.auth), audit_dir=str(self.audit), config=str(self.config),
            release=str(self.release), portable_factor=str(self.portable), snapshot_id="Fixture-20260920-1", output=str(self.output))

    @staticmethod
    def json(value):
        return (json.dumps(value, sort_keys=True) + "\n").encode()

    @staticmethod
    def write(path, raw, mode=0o600):
        path.write_bytes(raw)
        path.chmod(mode)

    def run_cli(self, *args, ok=True):
        result = subprocess.run([sys.executable, "-I", "-B", str(TOOL), *map(str, args)],
                                capture_output=True, text=True, timeout=45)
        self.assertEqual(result.returncode == 0, ok, result.stderr)
        for private in (FIXTURE, "fixture-subject", "fictional-digest", "fictional-code-digest"):
            self.assertNotIn(private, result.stdout + result.stderr)
        return result

    def create(self, ok=True):
        flags = [part for key, value in vars(self.args).items() for part in ("--" + key.replace("_", "-"), value)]
        return self.run_cli("create", *flags, ok=ok)

    def verify(self, ok=True):
        return self.run_cli("verify", self.output, ok=ok)

    def sources(self):
        paths = [p for root in (self.auth, self.audit, self.release) for p in [root, *root.rglob("*")]]
        paths += [self.config, self.portable]
        return {str(p): (MODULE["fp"](p.lstat()), None if p.is_dir() else p.read_bytes()) for p in paths}

    def sql(self, query, audit=False):
        path = (self.audit if audit else self.auth) / ("audit.sqlite3" if audit else "auth.sqlite3")
        with contextlib.closing(sqlite3.connect(path)) as db, db:
            db.execute(query)

    def manifest_edit(self, change):
        path = self.output / "manifest.json"
        value = json.loads(path.read_bytes())
        change(value)
        self.write(path, self.json(value))

    def rejected_incomplete(self):
        self.create(ok=False)
        self.assertTrue((self.output / "INCOMPLETE").is_file())
        self.verify(ok=False)
