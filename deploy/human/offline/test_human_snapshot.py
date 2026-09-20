#!/usr/bin/env python3
"""仅原生 Linux 的虚构快照契约测试；通过 unittest discover 运行整组。"""
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
    def test_roundtrip_complete_release_all_rows_and_unchanged_sources(self):
        before = self.sources()
        self.create()
        self.verify()
        self.assertEqual(before, self.sources())
        self.assertFalse((self.output / "INCOMPLETE").exists())
        manifest = json.loads((self.output / "manifest.json").read_bytes())
        self.assertEqual(manifest["snapshot_id"], self.args.snapshot_id)
        self.assertEqual((self.output / "owner.json").read_bytes(), self.config.read_bytes())
        self.assertEqual((self.output / "portable-factor.age").read_bytes(), self.portable.read_bytes())
        for name in set(self.release_manifest["files"]) | {"manifest.json"}:
            self.assertEqual((self.output / "release" / name).read_bytes(), (self.release / name).read_bytes())
        for folder, name in ((self.auth, "auth"), (self.audit, "audit")):
            with (contextlib.closing(sqlite3.connect(folder / (name + ".sqlite3"))) as source,
                  contextlib.closing(sqlite3.connect(self.output / name / (name + ".sqlite3"))) as target):
                self.assertEqual(list(source.iterdump()), list(target.iterdump()))
                self.assertEqual(target.execute("PRAGMA journal_mode").fetchone(), ("delete",))
        for path in [self.output, *self.output.rglob("*")]:
            expected = 0o700 if path.is_dir() else 0o600
            if path == self.output / "release/bin/studiolo-local-server":
                expected = 0o755
            elif path == self.output / "release/bin/studiolo-auth-admin":
                expected = 0o700
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), expected)
        self.assertEqual(set(manifest["files"]), {str(p.relative_to(self.output)) for p in self.output.rglob("*") if p.is_file()} - {"manifest.json"})

    def test_wal_backup_keeps_sources_including_shm_unchanged(self):
        db = sqlite3.connect(self.auth / "auth.sqlite3")
        self.addCleanup(db.close)
        db.execute("PRAGMA journal_mode=WAL")
        db.execute("PRAGMA wal_autocheckpoint=0")
        db.execute("INSERT INTO sessions VALUES(3,'wal-only-row')")
        db.commit()
        before = self.sources()
        self.create()
        self.verify()
        self.assertEqual(before, self.sources())
        with contextlib.closing(sqlite3.connect(self.output / "auth/auth.sqlite3")) as copied:
            self.assertEqual(copied.execute("SELECT marker FROM sessions WHERE id=3").fetchone(), ("wal-only-row",))

    def test_wrong_database_versions(self):
        for audit in (False, True):
            with self.subTest(audit=audit):
                self.sql("PRAGMA user_version=99", audit)
                self.rejected_incomplete()
                shutil.rmtree(self.output)
                self.sql("PRAGMA user_version=" + ("2" if audit else "4"), audit)

    def test_factor_must_be_v2_enrolled(self):
        for query in ("UPDATE totp_factor SET seed_ref='old-v1'", "UPDATE totp_factor SET enrollment_json=NULL",
                      "UPDATE totp_factor SET intent_receipt=NULL", "UPDATE totp_factor SET outcome_receipt=NULL",
                      "UPDATE totp_factor SET confirmed_at=NULL", "DELETE FROM totp_factor"):
            with self.subTest(query=query):
                raw = (self.auth / "auth.sqlite3").read_bytes()
                self.sql(query)
                self.rejected_incomplete()
                shutil.rmtree(self.output)
                self.write(self.auth / "auth.sqlite3", raw)

    def test_both_pending_tables_must_be_empty(self):
        for table in ("totp_pending", "recovery_pending"):
            with self.subTest(table=table):
                self.sql("INSERT INTO " + table + " VALUES(1)")
                self.rejected_incomplete()
                shutil.rmtree(self.output)
                self.sql("DELETE FROM " + table)

    def test_foreign_key_violation_and_corrupt_database(self):
        with contextlib.closing(sqlite3.connect(self.audit / "audit.sqlite3")) as db, db:
            db.executescript("CREATE TABLE children(id INTEGER REFERENCES records(id)); INSERT INTO children VALUES(999);")
        self.rejected_incomplete()
        shutil.rmtree(self.output)
        self.write(self.audit / "audit.sqlite3", b"not a SQLite database")
        self.rejected_incomplete()

    def test_config_binding_seed_and_unknown_fields(self):
        for key, value in (("auth_directory", str(self.audit)), ("audit_directory", str(self.auth)),
                           ("seed_ref", "studiolo-human-totp-v1"), ("unexpected", True), ("version", True)):
            with self.subTest(key=key):
                changed = dict(self.document, **{key: value})
                self.write(self.config, self.json(changed))
                self.rejected_incomplete()
                shutil.rmtree(self.output)

    def test_duplicate_config_keys(self):
        raw = self.json(self.document).replace(b'"version": 1', b'"version": 1, "version": 1')
        self.write(self.config, raw)
        self.rejected_incomplete()

    def test_snapshot_id_is_bounded_ascii(self):
        for value in ("", "a" * 65, "../bad", "bad/name", "non ascii\u00e9", "has_underscore"):
            with self.subTest(value=value):
                self.args.snapshot_id = value
                self.create(ok=False)
                self.assertFalse(self.output.exists())

    def test_cipher_format_size_and_mode(self):
        for raw, mode in ((b"-----BEGIN AGE ENCRYPTED FILE-----\n" + b"x" * 80, 0o600),
                          (b"age-encryption.org/v1\n", 0o600),
                          (b"age-encryption.org/v1\n" + b"x" * 65536, 0o600),
                          (self.portable.read_bytes(), 0o644)):
            with self.subTest(size=len(raw), mode=mode):
                self.write(self.portable, raw, mode)
                self.rejected_incomplete()
                shutil.rmtree(self.output)

    def test_source_root_and_config_private_modes(self):
        for path in (self.auth, self.audit, self.release, self.config.parent, self.config):
            with self.subTest(path=path.name):
                path.chmod(0o755 if path.is_dir() else 0o644)
                self.rejected_incomplete()
                shutil.rmtree(self.output)
                path.chmod(0o700 if path.is_dir() else 0o600)



if __name__ == "__main__":
    unittest.main()
