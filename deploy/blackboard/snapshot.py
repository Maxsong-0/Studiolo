"""发布前一致快照；仅复用原低权限 worker，数据库正文不输出。"""
import os
from pathlib import Path
import pwd
import sqlite3
import subprocess

NAMES = ('studiolo-human', 'studiolo-agent')


class SnapshotOperations:
    def worker(self, mode, descriptors=()):
        # 复用原 r8 快照 worker；代码允许 root 冻结、祖先仍可由 UID1000 拥有。
        # 不修改 r8 helper 的通用文件规则，也不读取认证数据到当前进程。
        c = self.c
        c.require(mode in ('binding', 'snapshot') and len(descriptors) == (3 if mode == 'snapshot' else 0))
        account = pwd.getpwnam('maxsong')
        c.require(account.pw_uid == c.UID == 1000)
        code = self.trusted_read(self.legacy.WORKER)
        c.require(len(code) <= 1024 * 1024)
        result = subprocess.run(['/usr/bin/python3', '-I', '-B', '-S', '-', mode,
            *(str(fd) for fd in descriptors)], input=code, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, pass_fds=tuple(descriptors), user=c.UID,
            group=account.pw_gid, extra_groups=[], cwd='/', env=self.runtime.ENV,
            timeout=40, check=True)
        c.require(len(result.stdout) <= 4096)
        value = c.document(result.stdout)
        c.require(set(value) == {'epoch', 'factor_ref', 'code_count', 'restore_event_count'})
        return value

    def backup(self):
        c = self.c
        self.stopped()
        self.unchanged()
        c.require({name: c.sha(raw) for name, raw in self.units().items()} == self.old_shas)
        for name in NAMES:
            c.new_file(self.record / (name + '.before.service'), self.old_units[name])
        c.new_file(self.record / 'owner.before.json', self.config)
        descriptors = []
        paths = [self.record / (name + '.sqlite3') for name in ('auth', 'audit', 'policy')]
        try:
            for path in paths:
                descriptors.append(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600))
            c.require(self.worker('snapshot', descriptors) == self.before)
        finally:
            for fd in descriptors:
                os.close(fd)
        files = {}
        for path in paths:
            c.inspect(path, 0, 0o600)
            db = sqlite3.connect(path.as_uri() + '?mode=ro&immutable=1', uri=True)
            try:
                c.require(db.execute('PRAGMA quick_check').fetchall() == [('ok',)])
                c.require(not db.execute('PRAGMA foreign_key_check').fetchall())
            finally:
                db.close()
        for path in paths + [self.record / (name + '.before.service') for name in NAMES] + [self.record / 'owner.before.json']:
            raw = c.read(path, 0, 0o600)
            files[path.name] = {'bytes': len(raw), 'sha256': c.sha(raw)}
        self.unchanged()
        self.receipt('backup', {'state': 'complete', 'files': files, 'sqlite_write_barrier': True,
                                'agent_identity_database_changed': False})

