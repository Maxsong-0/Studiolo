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


class SourceDirectoryTests(unittest.TestCase):
    """库目录白名单：只有审计库目录认准入锁，而且锁必须是空文件（vault BUG-040）。"""

    def setUp(self):
        self.mask = os.umask(0o077)
        self.temp = tempfile.TemporaryDirectory(prefix='.studiolo-r8-source-', dir=Path.home())
        self.root = Path(self.temp.name)

    def tearDown(self):
        self.temp.cleanup()
        os.umask(self.mask)

    def database(self, name):
        directory = self.root / name
        directory.mkdir(mode=0o700)
        path = directory / (name + '.sqlite3')
        db = sqlite3.connect(path)
        db.execute('CREATE TABLE fixture (value TEXT)')
        db.commit()
        db.close()
        return path

    @contextlib.contextmanager
    def owned_by_1000(self):
        # 测试箱里的 UID 不一定是 1000，临时目录的祖先也可能带 sticky 位；只换属主、去掉祖先的放宽位，
        # 名字、类型、大小、链接数、设备号都用真值。
        real = Path.lstat
        def lstat(path):
            s = real(path)
            inside = path == self.root or self.root in path.parents
            return types.SimpleNamespace(st_mode=s.st_mode if inside else s.st_mode & ~0o7022,
                st_uid=1000, st_nlink=s.st_nlink, st_size=s.st_size, st_dev=s.st_dev, st_ino=s.st_ino)
        with patch.object(Path, 'lstat', autospec=True, side_effect=lstat):
            yield

    def identity(self, path):
        s = path.stat()
        return s.st_dev, s.st_ino

    def test_gate_name_is_known_only_to_the_audit_directory(self):
        self.assertIn(worker.AUDIT_GATE, worker.expected_names(Path('/x/audit.sqlite3')))
        for name in ('auth.sqlite3', 'policy.sqlite3'):
            self.assertNotIn(worker.AUDIT_GATE, worker.expected_names(Path('/x') / name))

    def test_audit_directory_accepts_an_empty_gate_and_rejects_a_written_one(self):
        audit = self.database('audit')
        with self.owned_by_1000():
            self.assertEqual(worker.source(audit), self.identity(audit))
        gate = audit.parent / worker.AUDIT_GATE
        gate.touch(mode=0o600)
        with self.owned_by_1000():
            self.assertEqual(worker.source(audit), self.identity(audit))
        gate.write_bytes(b'x')
        with self.owned_by_1000(), self.assertRaises(ValueError):
            worker.source(audit)

    def test_gate_must_be_private(self):
        audit = self.database('audit')
        gate = audit.parent / worker.AUDIT_GATE
        gate.touch(mode=0o600)
        gate.chmod(0o640)
        with self.owned_by_1000(), self.assertRaises(ValueError):
            worker.source(audit)

    def test_gate_is_foreign_outside_the_audit_directory(self):
        auth = self.database('auth')
        (auth.parent / worker.AUDIT_GATE).touch(mode=0o600)
        with self.owned_by_1000(), self.assertRaises(ValueError):
            worker.source(auth)

    def test_other_stray_names_stay_rejected(self):
        audit = self.database('audit')
        for name in (worker.AUDIT_GATE + '.tmp', 'notes.md', 'auth.sqlite3'):
            stray = audit.parent / name
            stray.touch(mode=0o600)
            with self.subTest(name=name), self.owned_by_1000(), self.assertRaises(ValueError):
                worker.source(audit)
            stray.unlink()
