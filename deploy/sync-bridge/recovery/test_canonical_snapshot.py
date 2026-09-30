#!/usr/bin/python3
"""只用虚拟内容的三根恢复契约；真实 Broker 由 Rust 恢复探针覆盖。"""
import importlib.util
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest
import uuid

spec = importlib.util.spec_from_file_location('canonical_snapshot',
        Path(__file__).with_name('canonical_snapshot.py'))
subject = importlib.util.module_from_spec(spec)
spec.loader.exec_module(subject)


class SnapshotTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='.t132-fixture-', dir=Path.home())
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.roots = {name: self.base / name for name in subject.ROOTS}
        for root in self.roots.values():
            root.mkdir(mode=0o700)
        self.uuid = str(uuid.uuid4())
        self.binding = {'format_version': 1, 'vault_uuid': self.uuid,
                        'pair_id': str(uuid.uuid4())}
        control = self.roots['canonical'] / '.workbench'
        control.mkdir(mode=0o700)
        self.write(control / 'vault.json', {'format_version': 1, 'vault_uuid': self.uuid})
        self.write(control / 'broker-binding.json', self.binding)
        self.write(self.roots['writer'] / 'broker-binding.json', self.binding)
        (self.roots['canonical'] / 'note.md').write_text('fixture only\n')
        (self.roots['canonical'] / 'note.md').chmod(0o644)
        folder = self.roots['canonical'] / 'folder'
        folder.mkdir(mode=0o755)
        folder.chmod(0o755)
        with closing(sqlite3.connect(self.roots['board'] / 'control.sqlite3')) as db, db:
            db.executescript('CREATE TABLE vault_binding(singleton INTEGER, vault_uuid TEXT);'
                'CREATE TABLE schema_migrations(version INTEGER);'
                'INSERT INTO schema_migrations VALUES(1),(2),(3);')
            db.execute('INSERT INTO vault_binding VALUES(1, ?)', (self.uuid,))
            for name in ('tasks', 'attempts', 'snapshots', 'projection_outbox'):
                db.execute('CREATE TABLE ' + name + '(id INTEGER)')
        self.output = self.base / 'snapshot'

    def write(self, path, value):
        path.write_text(json.dumps(value))
        path.chmod(0o600)

    def freeze(self):
        return subject.freeze(self.roots, self.output, 'fixture-1', lambda: None)

    def test_copy_verify_restore_and_modes(self):
        old = os.umask(0o077)
        try:
            receipt = self.freeze()
            doc = subject.verify(self.output, receipt['manifest_sha256'])
            target = self.base / 'restored'
            subject.restore(self.output, receipt['manifest_sha256'], target)
            self.assertEqual(doc['vault_uuid'], self.uuid)
            for name in self.roots:
                self.assertEqual(subject.inventory(self.roots[name]),
                                 subject.inventory(target / name))
        finally:
            os.umask(old)

    def test_live_service_barrier_denies_before_copy(self):
        def live():
            raise ValueError('live')
        with self.assertRaises(ValueError):
            subject.freeze(self.roots, self.output, 'fixture', live)
        self.assertFalse(self.output.exists())

    def test_change_during_snapshot_never_gets_manifest(self):
        calls = []
        def barrier():
            calls.append(1)
            if len(calls) == 2:
                (self.roots['canonical'] / 'note.md').write_text('changed')
        with self.assertRaises(ValueError):
            subject.freeze(self.roots, self.output, 'fixture', barrier)
        self.assertTrue((self.output / 'INCOMPLETE').exists())
        self.assertFalse((self.output / 'manifest.json').exists())

    def test_closed_tamper_and_digest_checks(self):
        receipt = self.freeze()
        digest = receipt['manifest_sha256']
        with self.assertRaises(ValueError):
            subject.verify(self.output, '0' * 64)
        extra = self.output / 'extra'
        extra.touch()
        with self.assertRaises(ValueError):
            subject.verify(self.output, digest)
        extra.unlink()
        (self.output / 'canonical/note.md').write_text('tampered')
        with self.assertRaises(ValueError):
            subject.verify(self.output, digest)

    def test_links_and_writable_files_are_denied(self):
        target = self.roots['canonical'] / 'unsafe'
        for mode in ('symlink', 'hardlink', 'writable'):
            if mode == 'symlink':
                target.symlink_to('note.md')
            elif mode == 'hardlink':
                os.link(self.roots['canonical'] / 'note.md', target)
            else:
                target.touch(mode=0o666)
                target.chmod(0o666)
            with self.subTest(mode=mode), self.assertRaises(ValueError):
                self.freeze()
            target.unlink()
        alias = self.base / 'alias'
        alias.symlink_to(self.roots['canonical'], target_is_directory=True)
        with self.assertRaises(ValueError):
            subject.inventory(alias)

    def test_live_wal_and_wrong_board_identity_are_denied(self):
        wal = self.roots['board'] / 'control.sqlite3-wal'
        wal.touch()
        with self.assertRaises(ValueError):
            self.freeze()
        wal.unlink()
        with closing(sqlite3.connect(self.roots['board'] / 'control.sqlite3')) as db, db:
            db.execute('UPDATE vault_binding SET vault_uuid=?', (str(uuid.uuid4()),))
        with self.assertRaises(ValueError):
            self.freeze()

    def test_existing_and_overlapping_destinations_are_denied(self):
        for path in (self.roots['canonical'], self.roots['canonical'] / 'inside'):
            with self.subTest(path=path), self.assertRaises(ValueError):
                subject.freeze(self.roots, path, 'fixture', lambda: None)
        receipt = self.freeze()
        with self.assertRaises(ValueError):
            subject.restore(self.output, receipt['manifest_sha256'], self.output / 'inside')
        with self.assertRaises(ValueError):
            subject.restore(self.output, receipt['manifest_sha256'], self.roots['writer'])

    def test_manifest_cannot_be_symlink(self):
        receipt = self.freeze()
        manifest = self.output / 'manifest.json'
        real = self.base / 'elsewhere.json'
        manifest.rename(real)
        manifest.symlink_to(real)
        with self.assertRaises(ValueError):
            subject.verify(self.output, receipt['manifest_sha256'])

    @unittest.skipIf(os.geteuid() == 0, '需要普通用户触发权限拒绝')
    def test_unreadable_subtree_never_gets_manifest(self):
        blocked = self.roots['canonical'] / 'unreadable'
        blocked.mkdir(mode=0o700)
        (blocked / 'content.md').write_text('fixture')
        blocked.chmod(0)
        try:
            with self.assertRaises(PermissionError):
                self.freeze()
            self.assertFalse((self.output / 'manifest.json').exists())
        finally:
            blocked.chmod(0o700)


if __name__ == '__main__':
    unittest.main()
