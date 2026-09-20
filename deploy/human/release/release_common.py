"""固定人侧发行格式与不跟随链接的文件操作；不含服务操作。"""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import unicodedata
import argparse

BUILD = 'm4-t094-20260920-r8'
OLD_RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r7'
RELEASE = Path('/home/maxsong/.local/share/studiolo-human-releases') / BUILD
UID = 1000
MAX_FILE = 512 * 1024 * 1024
MAX_TOTAL = 2 * 1024 * 1024 * 1024
BINARIES = {'bin/studiolo-local-server', 'bin/studiolo-auth-admin'}
REQUIRED = BINARIES | {'launch-human.py', 'studiolo-human.service', 'web/index.html'}


def require(value):
    if not value:
        raise ValueError('rejected')


class ClosedParser(argparse.ArgumentParser):
    def error(self, _message):
        raise ValueError('invalid arguments')


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result)
        result[key] = value
    return result


def document(raw):
    require(len(raw) <= 4 * 1024 * 1024)
    return json.loads(raw, object_pairs_hook=unique)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def mark(s):
    return (s.st_dev, s.st_ino, s.st_uid, s.st_gid, s.st_mode, s.st_nlink,
            s.st_size, s.st_mtime_ns, s.st_ctime_ns)


def absolute(path):
    value = os.fspath(path)
    require(value.startswith('/') and not value.endswith('/') and len(value) <= 4096)
    require(all(x not in ('', '.', '..') for x in value.split('/')[1:]))
    return Path(value)


def ancestors(path, uid):
    for node in reversed(path.parents):
        s = node.lstat()
        require(stat.S_ISDIR(s.st_mode) and s.st_uid in (0, uid))
        require(stat.S_IMODE(s.st_mode) & 0o7022 == 0)


def inspect(path, uid, mode=None, directory=False):
    path = absolute(path)
    ancestors(path, uid)
    s = path.lstat()
    require(s.st_uid == uid and (stat.S_ISDIR(s.st_mode) if directory else
                              stat.S_ISREG(s.st_mode) and s.st_nlink == 1))
    require(stat.S_IMODE(s.st_mode) & 0o7022 == 0)
    if mode is not None:
        require(stat.S_IMODE(s.st_mode) == mode)
    if not directory:
        require(s.st_size <= MAX_FILE)
    return s


def read(path, uid, mode=None):
    before = inspect(path, uid, mode)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    try:
        require(mark(os.fstat(fd)) == mark(before))
        with os.fdopen(fd, 'rb', closefd=False) as stream:
            raw = stream.read(MAX_FILE + 1)
        require(len(raw) == before.st_size and mark(os.fstat(fd)) == mark(before))
        require(mark(inspect(path, uid, mode)) == mark(before))
        return raw
    finally:
        os.close(fd)


def relative(value):
    require(isinstance(value, str) and 0 < len(value) <= 4096)
    require(unicodedata.normalize('NFC', value) == value and '\\' not in value)
    require(not any(ord(c) < 32 or ord(c) == 127 for c in value))
    require(all(p not in ('', '.', '..') for p in value.split('/')))
    return Path(value)


def mode_for(name):
    return 0o755 if name in BINARIES else 0o600


def scan(root, uid, strict=False):
    root = absolute(root)
    origin = inspect(root, uid, 0o700 if strict else None, True)
    files, directories = {}, set()
    for parent, dirs, names in os.walk(root, followlinks=False):
        for name in dirs + names:
            path = Path(parent) / name
            rel = path.relative_to(root).as_posix()
            relative(rel)
            s = inspect(path, uid, directory=name in dirs)
            require(s.st_dev == origin.st_dev)
            if name in dirs:
                if strict:
                    require(stat.S_IMODE(s.st_mode) == 0o700)
                directories.add(rel)
            else:
                if strict:
                    require(stat.S_IMODE(s.st_mode) == mode_for(rel))
                raw = read(path, uid)
                files[rel] = {'bytes': len(raw), 'sha256': sha(raw)}
            require(len(files) + len(directories) <= 16384)
    require(sum(item['bytes'] for item in files.values()) <= MAX_TOTAL)
    require(mark(root.lstat()) == mark(origin))
    return files, directories


def verify(root, expected_sha, uid=UID):
    require(re.fullmatch('[0-9a-f]{64}', expected_sha) is not None)
    raw = read(root / 'manifest.json', uid, 0o600)
    require(sha(raw) == expected_sha)
    manifest = document(raw)
    require(set(manifest) == {'build', 'source_commit', 'assembly', 'files'})
    require(manifest['build'] == BUILD and manifest['assembly'] == 'human-v2-r8')
    require(re.fullmatch('[0-9a-f]{40}', manifest['source_commit']) is not None)
    files = manifest['files']
    require(isinstance(files, dict) and REQUIRED <= files.keys())
    require(any(name.startswith('example-vault/') for name in files))
    directories = set()
    for name, item in files.items():
        path = relative(name)
        require(name != 'manifest.json' and (name in REQUIRED or
                name.startswith('web/') or name.startswith('example-vault/')))
        require(set(item) == {'bytes', 'sha256'} and type(item['bytes']) is int)
        require(0 <= item['bytes'] <= MAX_FILE and re.fullmatch('[0-9a-f]{64}', item['sha256']))
        directories.update(p.as_posix() for p in path.parents if str(p) != '.')
    actual, dirs = scan(root, uid, True)
    require(dirs == directories and set(actual) == set(files) | {'manifest.json'})
    require(all(actual[name] == item for name, item in files.items()))
    return manifest


def new_file(path, data, mode=0o600):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, mode)
    try:
        with os.fdopen(fd, 'wb', closefd=False) as stream:
            os.fchmod(fd, mode)
            stream.write(data)
            stream.flush()
            os.fsync(fd)
    finally:
        os.close(fd)


def atomic_unit(path, data, expected_sha):
    require(sha(read(path, 0, 0o644)) == expected_sha)
    fd, temporary = tempfile.mkstemp(prefix='.human-r8-', dir=path.parent)
    try:
        with os.fdopen(fd, 'wb') as stream:
            os.fchmod(stream.fileno(), 0o644)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        require(sha(read(path, 0, 0o644)) == expected_sha)
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if os.path.lexists(temporary):
            os.unlink(temporary)
