"""使用真实私有目录验证冻结锁、完整性和不覆盖；无生产访问。"""
import fcntl
import json
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import replica_common as c
import replica_store as store
import freeze_replica as f


def private_file(path, raw):
    path.write_bytes(raw)
    path.chmod(0o600)


class FreezeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.source = self.root / 'replica'
        self.output = self.root / 'frozen'
        self.generation = self.source / 'releases' / ('a' * 32)
        for directory in (self.source, self.source / 'control', self.source / 'releases',
                          self.generation, self.generation / 'vault'):
            directory.mkdir(mode=0o700)
        entries = []
        for scope in f.ALLOW:
            target = self.generation / 'vault' / scope
            target.mkdir(mode=0o700, parents=True)
            for directory in target.parents:
                if directory == self.root:
                    break
                directory.chmod(0o700)
            raw = b'# frozen fixture\n'
            private_file(target / 'note.md', raw)
            entries.append({'path': scope + '/note.md', 'kind': 'file',
                            'bytes': len(raw), 'sha256': c.digest(raw)})
        manifest = c.encode({'schema_version': 1, 'allow': f.ALLOW,
                             'entries': sorted(entries, key=lambda x: x['path'])})
        private_file(self.generation / 'manifest.json', manifest)
        now = time.time_ns() // 1_000_000
        self.state = {'schema_version': 1, 'status': 'ready', 'last_attempt_at_ms': now,
                      'last_error': None, 'receipt': {'generation': 'a' * 32,
                          'source_digest': c.digest(manifest), 'source_observed_at_ms': now,
                          'received_at_ms': now}}
        self.save_state()
        private_file(self.source / 'sync.lock', b'')
        for module, name, value in ((f, 'SOURCE', str(self.source)), (f, 'OUTPUT', str(self.output)),
                                    (c, 'ALLOW', f.ALLOW)):
            item = patch.object(module, name, value)
            item.start()
            self.addCleanup(item.stop)

    def save_state(self):
        private_file(self.source / 'control/state.json', c.encode(self.state))

    def test_freeze_copies_verified_bytes_and_keeps_source_state(self):
        state = (self.source / 'control/state.json').read_bytes()
        result = f.freeze(c, store)
        self.assertEqual(result['files'], 3)
        self.assertFalse(result['continuous_sync'])
        self.assertEqual(state, (self.source / 'control/state.json').read_bytes())
        for node in self.generation.rglob('*'):
            target = self.output / node.relative_to(self.generation)
            self.assertEqual(target.stat().st_mode & 0o777, 0o700 if node.is_dir() else 0o600)
            if node.is_file():
                self.assertEqual(node.read_bytes(), target.read_bytes())
        with self.assertRaises(FileExistsError):
            f.freeze(c, store)
        self.assertEqual(state, (self.source / 'control/state.json').read_bytes())

    def test_stale_or_corrupt_source_cannot_create_snapshot(self):
        self.state['receipt']['received_at_ms'] -= 301_000
        self.state['receipt']['source_observed_at_ms'] -= 301_000
        self.save_state()
        with self.assertRaises(c.Invalid):
            f.freeze(c, store)
        self.assertFalse(self.output.exists())
        self.state['receipt']['received_at_ms'] += 301_000
        self.state['receipt']['source_observed_at_ms'] += 301_000
        self.save_state()
        private_file(self.generation / 'vault/项目/Studiolo/note.md', b'changed')
        with self.assertRaises(c.Invalid):
            f.freeze(c, store)
        self.assertFalse(self.output.exists())

    def test_publisher_lock_refuses_without_writing(self):
        with (self.source / 'sync.lock').open('rb') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                f.freeze(c, store)
        self.assertFalse(self.output.exists())


if __name__ == '__main__':
    unittest.main()
