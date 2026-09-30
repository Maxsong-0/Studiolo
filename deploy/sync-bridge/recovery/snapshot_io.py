"""三根恢复点的闭集文件复制；拒绝链接、特殊文件与读取期间变化。"""
import hashlib
import os
from pathlib import Path
import stat

MAX_FILES = 100000
MAX_BYTES = 8 * 1024 ** 3


def require(value):
    if not value:
        raise ValueError('snapshot invariant')


def mark(s):
    return (s.st_dev, s.st_ino, s.st_mode, s.st_uid, s.st_gid,
            s.st_nlink, s.st_size, s.st_mtime_ns, s.st_ctime_ns)


def directory(path):
    path = Path(path)
    require(path.is_absolute() and '..' not in path.parts)
    for p in [*reversed(path.parents), path]:
        s = p.lstat()
        require(stat.S_ISDIR(s.st_mode) and not s.st_mode & 0o022)
    return path


def inventory(root, copy_to=None):
    root = directory(root)
    root_before = mark(root.lstat())
    require(root.lstat().st_uid == os.geteuid())
    result = {}
    total = 0
    if copy_to is not None:
        directory(copy_to.parent)
        copy_to.mkdir(mode=0o700)
    def walk_error(error):
        raise error
    for parent, dirs, files in os.walk(root, followlinks=False, onerror=walk_error):
        directory(Path(parent))
        dirs.sort()
        files.sort()
        for name in dirs + files:
            source = Path(parent) / name
            relative = source.relative_to(root).as_posix()
            require(len(result) < MAX_FILES and '\n' not in relative)
            before = source.lstat()
            mode = stat.S_IMODE(before.st_mode)
            require(not mode & 0o7022 and before.st_uid == os.geteuid())
            destination = copy_to / relative if copy_to is not None else None
            if stat.S_ISDIR(before.st_mode):
                result[relative] = {'kind': 'directory', 'mode': mode}
                if destination is not None:
                    destination.mkdir(mode=mode)
                    destination.chmod(mode)
                continue
            require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1)
            total += before.st_size
            require(total <= MAX_BYTES)
            flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK
            fd = os.open(source, flags)
            output = None
            try:
                require(mark(os.fstat(fd)) == mark(before))
                if destination is not None:
                    out = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL
                                  | os.O_NOFOLLOW | os.O_CLOEXEC, mode)
                    os.fchmod(out, mode)
                    output = os.fdopen(out, 'wb')
                digest = hashlib.sha256()
                count = 0
                with os.fdopen(fd, 'rb', closefd=False) as stream:
                    for data in iter(lambda: stream.read(1024 * 1024), b''):
                        count += len(data)
                        require(count <= before.st_size)
                        digest.update(data)
                        if output is not None:
                            output.write(data)
                require(count == before.st_size)
                require(mark(os.fstat(fd)) == mark(before)
                        and mark(source.lstat()) == mark(before))
                if output is not None:
                    output.flush()
                    os.fsync(output.fileno())
                result[relative] = {'kind': 'file', 'mode': mode,
                                    'bytes': count, 'sha256': digest.hexdigest()}
            finally:
                if output is not None:
                    output.close()
                os.close(fd)
    require(mark(root.lstat()) == root_before)
    return result


def read_regular(path, limit):
    directory(path.parent)
    before = path.lstat()
    require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1
            and before.st_uid == os.geteuid() and not before.st_mode & 0o7022
            and before.st_size <= limit)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as stream:
        require(mark(os.fstat(stream.fileno())) == mark(before))
        raw = stream.read(limit + 1)
        require(len(raw) == before.st_size and len(raw) <= limit)
        require(mark(os.fstat(stream.fileno())) == mark(before)
                and mark(path.lstat()) == mark(before))
    return raw


def new_json(path, value):
    import json
    raw = (json.dumps(value, sort_keys=True, ensure_ascii=False) + '\n').encode()
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL
                 | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    with os.fdopen(fd, 'wb') as f:
        f.write(raw)
        f.flush()
        os.fsync(f.fileno())
    return hashlib.sha256(raw).hexdigest()
