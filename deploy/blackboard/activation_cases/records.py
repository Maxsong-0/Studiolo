"""records 发行契约。"""
from .fixture import *


class RecordsTests(FilesBase):
    def test_record_parent_0750_keeps_private_record_and_lock(self):
        directory = self.root / 'records'
        directory.mkdir(mode=0o700)
        directory.chmod(0o750)
        prior = directory / 'prior-group-visible'
        prior.mkdir(mode=0o700)
        prior.chmod(0o750)
        self.write(prior / 'receipt', b'preserve previous deployment')
        before = (prior.stat().st_mode, (prior / 'receipt').read_bytes())
        ops = self.begin_record(directory)
        try:
            self.assertEqual(directory.stat().st_mode & 0o7777, 0o750)
            self.assertEqual(ops.record.stat().st_mode & 0o7777, 0o700)
            self.assertEqual((directory / '.board-r1.lock').stat().st_mode & 0o7777, 0o600)
            self.assertEqual(ops.record.stat().st_uid, self.uid)
            self.assertEqual((prior.stat().st_mode, (prior / 'receipt').read_bytes()), before)
        finally:
            ops.close()

    def test_record_parent_group_or_other_write_refused_before_creation(self):
        for mode in (0o770, 0o752):
            with self.subTest(mode=oct(mode)):
                directory = self.root / ('records-' + oct(mode))
                directory.mkdir(mode=0o700)
                directory.chmod(mode)
                with self.assertRaises(ValueError):
                    self.begin_record(directory)
                self.assertEqual(list(directory.iterdir()), [])
                self.assertEqual(directory.stat().st_mode & 0o7777, mode)

    def test_record_parent_link_refused_without_touching_target(self):
        target = self.root / 'records-target'
        target.mkdir(mode=0o700)
        link = self.root / 'records-link'
        link.symlink_to(target, target_is_directory=True)
        with self.assertRaises(ValueError):
            self.begin_record(link)
        self.assertEqual(list(target.iterdir()), [])
        self.assertTrue(link.is_symlink())

    def test_existing_group_readable_lock_remains_rejected(self):
        directory = self.root / 'records'
        directory.mkdir(mode=0o700)
        directory.chmod(0o750)
        lock = directory / '.board-r1.lock'
        self.write(lock, b'', 0o640)
        with self.assertRaises(ValueError):
            self.begin_record(directory)
        self.assertEqual(list(directory.iterdir()), [lock])
        self.assertEqual(lock.stat().st_mode & 0o7777, 0o640)

    def test_agent_read_accepts_only_approved_0550_mode(self):
        binary = self.root / 'studiolo-agent'
        self.write(binary, b'fictional immutable agent', 0o550)
        ops = Operations('0' * 64, assembly, c, Mock(), legacy)
        ops.agent_binary = binary
        self.assertEqual(ops.read_agent(), b'fictional immutable agent')
        # 0700（未冻结、UID1000 可改写）明确拒绝：2026-09-22 T-101 带出过一次，已冻结回 0550。
        for mode in (0o700, 0o755, 0o750, 0o555, 0o500, 0o440, 0o570, 0o770, 0o707, 0o600):
            with self.subTest(mode=oct(mode)):
                binary.chmod(mode)
                with self.assertRaises(ValueError):
                    ops.read_agent()

