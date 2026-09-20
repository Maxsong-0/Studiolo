"""真实虚构SQLite写屏障；仅注入身份/固定路径，不读取任何生产库。"""
import contextlib
import os
from pathlib import Path
import sqlite3
import tempfile
import types
import unittest
from unittest.mock import patch
import snapshot_worker as worker


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.mask = os.umask(0o077)
        self.temp = tempfile.TemporaryDirectory(prefix='.studiolo-r8-sqlite-', dir=Path.home())
        self.root, self.connections, self.descriptors = Path(self.temp.name), [], []
        self.databases = tuple((self.root / (name + '.sqlite3'), version)
                               for name, version in (('auth', 4), ('audit', 2), ('policy', 1)))
        for path, version in self.databases:
            db = sqlite3.connect(path)
            db.execute('PRAGMA journal_mode=WAL')
            db.execute('PRAGMA user_version=' + str(version))
            db.execute('CREATE TABLE fixture (value TEXT)')
            if version == 4:
                db.executescript('''
                CREATE TABLE totp_factor(seed_ref TEXT,enrollment_json TEXT);
                INSERT INTO totp_factor VALUES('studiolo-human-totp-v2','fictional');
                CREATE TABLE totp_pending(id INTEGER);
                CREATE TABLE recovery_pending(id INTEGER);
                CREATE TABLE recovery_control(change_json TEXT);
                INSERT INTO recovery_control VALUES('fictional');
                CREATE TABLE auth_meta(id INTEGER,epoch INTEGER);
                INSERT INTO auth_meta VALUES(1,7);
                CREATE TABLE recovery_codes(id INTEGER);
                CREATE TABLE audit_outbox(event_json TEXT);
                ''')
                db.executemany('INSERT INTO recovery_codes VALUES(?)', [(x,) for x in range(10)])
            db.commit()
            db.execute('PRAGMA wal_checkpoint(TRUNCATE)')
            db.execute("INSERT INTO fixture VALUES('fictional WAL value')")
            db.commit()
            self.connections.append(db)
        for name in ('auth', 'audit', 'policy'):
            self.descriptors.append(os.open(self.root / (name + '.copy'), os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600))

    def tearDown(self):
        for descriptor in self.descriptors:
            os.close(descriptor)
        for db in self.connections:
            db.close()
        self.temp.cleanup()
        os.umask(self.mask)

    @contextlib.contextmanager
    def fictional_identity(self):
        real_stat = os.fstat
        def root_output(fd):
            original = real_stat(fd)
            return types.SimpleNamespace(st_mode=original.st_mode, st_uid=0,
                st_nlink=original.st_nlink, st_size=original.st_size)
        def identity(path):
            original = path.stat()
            return original.st_dev, original.st_ino
        with patch.object(worker, 'DATABASES', self.databases), \
                patch.object(worker.os, 'getuid', return_value=1000), \
                patch.object(worker.os, 'geteuid', return_value=1000), \
                patch.object(worker.os, 'fstat', side_effect=root_output), \
                patch.object(worker, 'source', side_effect=identity), \
                patch.object(worker.resource, 'setrlimit'):
            yield

    def test_wal_snapshot_uses_all_write_locks_then_distinct_readers(self):
        events, real_connect = [], worker.connect
        def connect(path, writable=False):
            events.append((path.name, writable))
            return real_connect(path, writable)
        with self.fictional_identity(), patch.object(worker, 'connect', side_effect=connect):
            report = worker.run('snapshot', self.descriptors)
        self.assertEqual([writable for _, writable in events], [True, True, True, False, False, False])
        self.assertEqual(report, dict(epoch=7, factor_ref='studiolo-human-totp-v2',
                                    code_count=10, restore_event_count=0))
        for (path, version), original in zip(self.databases, self.connections):
            copy = self.root / (path.stem + '.copy')
            db = sqlite3.connect(copy.as_uri() + '?mode=ro&immutable=1', uri=True)
            try:
                self.assertEqual(db.execute('PRAGMA user_version').fetchone(), (version,))
                self.assertEqual(db.execute('SELECT value FROM fixture').fetchall(), [('fictional WAL value',)])
            finally:
                db.close()
            original.execute('BEGIN IMMEDIATE')
            original.rollback()

    def test_lock_failure_releases_prior_lock_and_keeps_empty_outputs(self):
        self.connections[1].execute('BEGIN IMMEDIATE')
        with self.fictional_identity(), self.assertRaises(sqlite3.OperationalError):
            worker.run('snapshot', self.descriptors)
        self.connections[0].execute('BEGIN IMMEDIATE')
        self.connections[0].rollback()
        self.connections[1].rollback()
        self.assertTrue(all(os.fstat(fd).st_size == 0 for fd in self.descriptors))

    def test_unknown_version_is_rejected_before_output(self):
        self.connections[0].execute('PRAGMA user_version=99')
        with self.fictional_identity(), self.assertRaises(ValueError):
            worker.run('snapshot', self.descriptors)
        self.assertTrue(all(os.fstat(fd).st_size == 0 for fd in self.descriptors))
