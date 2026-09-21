#!/usr/bin/python3
"""在既有发布锁内验证并冻结一次只读副本；不将它当作持续 Sync。"""
import fcntl
import json
import os
from pathlib import Path
import stat
import sys
import time

ROOT = Path(os.path.abspath(__file__)).parents[2]
SCRIPTS = ROOT / 'scripts'
SOURCE = '/home/maxsong/.local/share/studiolo-agent-replica'
OUTPUT = '/home/maxsong/.local/share/studiolo-bootstrap-snapshot-20260920-r1'
ALLOW = sorted(['项目/Studiolo', '项目/Tillerwright', '_规则'])


def imports():
    for directory in (SCRIPTS, *SCRIPTS.parents):
        s = directory.lstat()
        if not stat.S_ISDIR(s.st_mode) or s.st_uid not in (0, os.geteuid()) or s.st_mode & 0o7022:
            raise ValueError('unsafe tools')
    for name in ('replica_common.py', 'replica_store.py', 'replica_cleanup.py'):
        s = (SCRIPTS / name).lstat()
        if not stat.S_ISREG(s.st_mode) or s.st_nlink != 1 or s.st_uid not in (0, os.geteuid()) or s.st_mode & 0o7022:
            raise ValueError('unsafe tools')
    sys.path.insert(0, str(SCRIPTS))
    import replica_common as c
    import replica_store as store
    # 此进程只验证明确的线上三项范围，不改变 receiver 或其发布状态。
    c.ALLOW = ALLOW
    return c, store


def freeze(c, store):
    fds = []
    def keep(fd):
        fds.append(fd)
        return fd
    try:
        root = keep(c.open_root(SOURCE, private=True))
        lock = keep(os.open('sync.lock', os.O_RDONLY | os.O_NOFOLLOW, dir_fd=root))
        c.check_node(os.fstat(lock), False, private=True)
        fcntl.flock(lock, fcntl.LOCK_SH | fcntl.LOCK_NB)
        control = keep(c.open_dir(root, 'control'))
        raw_state, _ = c.read_file(control, 'state.json', 8192)
        state = store.state_valid(c.load_json(raw_state))
        c.require(state['status'] == 'ready')
        receipt = state['receipt']
        c.require(0 <= time.time_ns() // 1_000_000 - receipt['received_at_ms'] <= 300_000)
        generation = keep(c.descend(root, 'releases/' + receipt['generation']))
        raw_manifest, _ = c.read_file(generation, 'manifest.json', c.MAX_MANIFEST)
        _, expected = c.manifest(raw_manifest)
        c.require(store.verify_tree(generation, expected) == receipt['source_digest'])
        parent, name = OUTPUT.rsplit('/', 1)
        parent_fd = keep(c.open_root(parent))
        os.mkdir(name, 0o700, dir_fd=parent_fd)  # 必须全新，失败保留现场。
        output = keep(c.open_dir(parent_fd, name))
        c.write_file(output, 'manifest.json', raw_manifest)
        files = 0
        for path, (kind, size, digest) in sorted(expected.items()):
            if kind == 'dir':
                opened = c.descend(output, path, create=True)
                os.close(opened)
            else:
                directory, filename = path.rsplit('/', 1)
                source_fd = c.descend(generation, directory)
                target_fd = c.descend(output, directory)
                try:
                    raw, _ = c.read_file(source_fd, filename, c.MAX_FILE)
                    c.require(len(raw) == size and c.digest(raw) == digest)
                    c.write_file(target_fd, filename, raw)
                    files += 1
                finally:
                    os.close(source_fd)
                    os.close(target_fd)
        c.require(store.verify_tree(output, expected) == receipt['source_digest'])
        c.require(store.verify_tree(generation, expected) == receipt['source_digest'])
        c.require(c.read_file(control, 'state.json', 8192)[0] == raw_state)
        c.require(os.fstat(lock).st_ino == os.stat('sync.lock', dir_fd=root, follow_symlinks=False).st_ino)
        os.fsync(output)
        os.fsync(parent_fd)
        return {'source_digest': receipt['source_digest'], 'generation': receipt['generation'],
                'source_observed_at_ms': receipt['source_observed_at_ms'], 'files': files,
                'allow': ALLOW, 'continuous_sync': False}
    finally:
        for fd in reversed(fds):
            os.close(fd)


if __name__ == '__main__':
    try:
        if len(sys.argv) != 1:
            raise ValueError('no arguments')
        os.umask(0o077)
        print(json.dumps(freeze(*imports()), ensure_ascii=False, sort_keys=True))
    except Exception:
        print('freeze rejected; any partial snapshot retained', file=sys.stderr)
        raise SystemExit(1)
