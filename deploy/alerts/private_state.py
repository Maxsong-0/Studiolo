"""只读秘密文件和私有状态：固定文件名、进程锁、原子替换，不纠正不安全权限。"""
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import stat
import uuid


def closed_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError("duplicate_state_key")
        value[key] = item
    return value


def reject_constant(value):
    raise ValueError("invalid_state_constant")


def private_directory(path):
    parts = Path(path).parts
    if not parts or parts[0] != "/" or ".." in parts:
        raise ValueError("unsafe_storage")
    fd = os.open("/", os.O_RDONLY | os.O_DIRECTORY | os.O_CLOEXEC)
    try:
        for part in parts[1:]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC, dir_fd=fd)
            os.close(fd)
            fd = child
            info = os.fstat(fd)
            sticky_root = info.st_uid == 0 and info.st_mode & stat.S_ISVTX
            if info.st_uid not in (0, os.getuid()) or (info.st_mode & 0o022 and not sticky_root):
                raise ValueError("unsafe_storage")
        info = os.fstat(fd)
        if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
            raise ValueError("unsafe_storage")
        return fd
    except BaseException:
        os.close(fd)
        raise


def private_file(fd):
    info = os.fstat(fd)
    if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1:
        raise ValueError("unsafe_storage")


def read_secret(path):
    parent = private_directory(str(Path(path).parent))
    try:
        fd = os.open(Path(path).name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, dir_fd=parent)
        with os.fdopen(fd, "rb") as source:
            private_file(source.fileno())
            raw = source.read(4097)
        value = raw.decode().strip()
        if not value or len(raw) > 4096 or any(char.isspace() for char in value):
            raise ValueError("secret_invalid")
        return value
    finally:
        os.close(parent)


class State:
    def __init__(self, directory):
        self.directory = directory

    def load(self):
        try:
            fd = os.open("state.json", os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, dir_fd=self.directory)
        except FileNotFoundError:
            return None
        with os.fdopen(fd, "rb") as source:
            private_file(source.fileno())
            raw = source.read(16385)
        if len(raw) > 16384:
            raise ValueError("state_too_large")
        return json.loads(raw, object_pairs_hook=closed_object, parse_constant=reject_constant)

    def save(self, value):
        raw = json.dumps(value, separators=(",", ":"), allow_nan=False).encode()
        if len(raw) > 16384:
            raise ValueError("state_too_large")
        name = ".state-" + uuid.uuid4().hex
        fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600, dir_fd=self.directory)
        try:
            with os.fdopen(fd, "wb") as target:
                private_file(target.fileno())
                target.write(raw)
                target.flush()
                os.fsync(target.fileno())
            os.replace(name, "state.json", src_dir_fd=self.directory, dst_dir_fd=self.directory)
            os.fsync(self.directory)
        finally:
            try:
                os.unlink(name, dir_fd=self.directory)
            except FileNotFoundError:
                pass


@contextmanager
def locked_state(path):
    directory = private_directory(path)
    lock = None
    try:
        lock = os.open("lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, 0o600, dir_fd=directory)
        private_file(lock)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield State(directory)
    finally:
        if lock is not None:
            os.close(lock)
        os.close(directory)
