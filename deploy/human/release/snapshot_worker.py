"""仅以UID1000运行：三库写屏障、独立只读连接backup、私有继承FD输出。"""
import json
import os
from pathlib import Path
import resource
import sqlite3
import stat
import sys
import time

ROOT = Path('/home/maxsong/.local/share')
DATABASES = ((ROOT / 'studiolo-human-auth/auth.sqlite3', 4),
             (ROOT / 'studiolo-human-audit/audit.sqlite3', 2),
             (ROOT / 'studiolo-agent-policy/policy.sqlite3', 1))
LIMIT = 512 * 1024 * 1024


def require(ok):
    if not ok:
        raise ValueError('rejected')


def source(path):
    for p in path.parents:
        s = p.lstat()
        require(stat.S_ISDIR(s.st_mode) and s.st_uid in (0, 1000) and s.st_mode & 0o7022 == 0)
    parent = path.parent.lstat()
    require(parent.st_uid == 1000 and stat.S_IMODE(parent.st_mode) == 0o700)
    allowed = {path.name + suffix for suffix in ('', '-wal', '-shm', '-journal')}
    require({p.name for p in path.parent.iterdir()} <= allowed)
    for p in path.parent.iterdir():
        s = p.lstat()
        require(stat.S_ISREG(s.st_mode) and s.st_uid == 1000 and s.st_nlink == 1)
        require(stat.S_IMODE(s.st_mode) == 0o600 and s.st_dev == parent.st_dev and s.st_size <= LIMIT)
    s = path.lstat()
    require(s.st_size > 0)
    return s.st_dev, s.st_ino


def connect(path, writable=False):
    connection = sqlite3.connect(path.as_uri() + ('?mode=rw' if writable else '?mode=ro'), timeout=1, uri=True)
    connection.execute('PRAGMA trusted_schema=OFF')
    if not writable:
        connection.execute('PRAGMA query_only=ON')
    return connection


def summary(db):
    require(db.execute('PRAGMA user_version').fetchone() == (4,))
    require(db.execute('SELECT seed_ref,enrollment_json IS NOT NULL FROM totp_factor').fetchall()
            == [('studiolo-human-totp-v2', 1)])
    for name in ('totp_pending', 'recovery_pending'):
        require(db.execute('SELECT count(*) FROM ' + name).fetchone() == (0,))
    require(db.execute('SELECT change_json IS NOT NULL FROM recovery_control').fetchone() == (1,))
    epoch = db.execute('SELECT epoch FROM auth_meta WHERE id=1').fetchone()[0]
    codes = db.execute('SELECT count(*) FROM recovery_codes').fetchone()[0]
    restored = db.execute("SELECT count(*) FROM audit_outbox WHERE json_extract(event_json,'$.payload.scope')='restore'").fetchone()[0]
    require(type(epoch) is int and epoch > 0 and codes in (0, 10) and restored == 0)
    return {'epoch': epoch, 'factor_ref': 'studiolo-human-totp-v2',
            'code_count': codes, 'restore_event_count': restored}


def run(mode, descriptors):
    require(os.getuid() == os.geteuid() == 1000)
    require(mode in ('binding', 'snapshot') and len(descriptors) == (3 if mode == 'snapshot' else 0))
    require(hasattr(sqlite3.Connection, 'serialize'))
    os.umask(0o077)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    end = time.monotonic() + 25
    identities = [source(path) for path, _ in DATABASES]
    locks = []
    try:
        if mode == 'snapshot':
            for fd in descriptors:
                s = os.fstat(fd)
                require(stat.S_ISREG(s.st_mode) and s.st_uid == 0 and s.st_nlink == 1
                        and stat.S_IMODE(s.st_mode) == 0o600 and s.st_size == 0)
            # 三库全部取得写屏障后才开始任何快照；不在锁连接上调用backup。
            for path, _ in DATABASES:
                lock = connect(path, True)
                locks.append(lock)
                lock.execute('BEGIN IMMEDIATE')
        total = 0
        result = None
        for index, (path, version) in enumerate(DATABASES):
            connection = connect(path)
            memory = None
            try:
                require(connection.execute('PRAGMA user_version').fetchone() == (version,))
                if index == 0:
                    result = summary(connection)
                if mode == 'snapshot':
                    memory = sqlite3.connect(':memory:')
                    def progress(*_):
                        require(time.monotonic() < end)
                    connection.backup(memory, pages=256, progress=progress, sleep=0.01)
                    require(memory.execute('PRAGMA quick_check').fetchall() == [('ok',)])
                    require(not memory.execute('PRAGMA foreign_key_check').fetchall())
                    raw = memory.serialize()
                    total += len(raw)
                    require(len(raw) <= LIMIT and total <= 1024 * 1024 * 1024)
                    with os.fdopen(descriptors[index], 'wb', closefd=False) as out:
                        out.write(raw)
                        out.flush()
                        os.fsync(out.fileno())
                    del raw
                require(time.monotonic() < end and source(path) == identities[index])
            finally:
                if memory is not None:
                    memory.close()
                connection.close()
        return result
    finally:
        for lock in reversed(locks):
            try:
                lock.rollback()
            finally:
                lock.close()


if __name__ == '__main__':
    try:
        result = run(sys.argv[1], [int(value) for value in sys.argv[2:]])
        print(json.dumps(result, sort_keys=True))
    except BaseException:
        print('snapshot worker rejected', file=sys.stderr)
        raise SystemExit(1)
