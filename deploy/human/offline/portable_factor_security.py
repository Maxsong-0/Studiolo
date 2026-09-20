"""portable 管线的 root 路径、描述符、进程保护与不覆盖发布边界。"""
import ctypes
import os
import resource
import secrets
import stat
import sys
import unicodedata

MAX_CIPHER = 65536
DIR_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC


def require(ok):
    if not ok:
        raise ValueError("rejected")


def absolute(path):
    require(type(path) is str and path.startswith("/") and path != "/" and len(path.encode()) <= 4096)
    require(len(path.split("/")) <= 64 and all(p not in ("", ".", "..") for p in path.split("/")[1:]))
    require("\\" not in path and unicodedata.normalize("NFC", path) == path)
    require(not any(unicodedata.category(c).startswith("C") for c in path))
    return path


def fingerprint(value):
    return (value.st_dev, value.st_ino, value.st_mode, value.st_uid, value.st_gid,
            value.st_nlink, value.st_size, value.st_mtime_ns, value.st_ctime_ns)


def protect():
    require(sys.platform == "linux" and os.getuid() == os.geteuid() == 0)
    require(sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    require(resource.getrlimit(resource.RLIMIT_CORE) == (0, 0))
    libc = ctypes.CDLL(None, use_errno=True)
    libc.prctl.argtypes = [ctypes.c_int] + [ctypes.c_ulong] * 4
    libc.prctl.restype = ctypes.c_int
    require(libc.prctl(4, 0, 0, 0, 0) == 0 and libc.prctl(3, 0, 0, 0, 0) == 0)
    require(libc.prctl(38, 1, 0, 0, 0) == 0 and libc.prctl(39, 0, 0, 0, 0) == 1)
    os.umask(0o077)


class Directory:
    def __init__(self, path, stack, private=False):
        absolute(path)
        fd = os.open("/", DIR_FLAGS)
        try:
            parts = path.split("/")[1:]
            for index, part in enumerate(parts):
                child = os.open(part, DIR_FLAGS, dir_fd=fd)
                os.close(fd)
                fd = child
                item = os.fstat(fd)
                mode = stat.S_IMODE(item.st_mode)
                require(item.st_uid == 0 and not mode & 0o6000)
                require(not mode & 0o022 or (index != len(parts) - 1 and mode & stat.S_ISVTX))
            require(not private or stat.S_IMODE(item.st_mode) == 0o700)
        except BaseException:
            os.close(fd)
            raise
        self.fd, self.path, self.before = fd, path, os.fstat(fd)
        stack.callback(os.close, fd)

    def bound(self, stack):
        other = Directory(self.path, stack)
        current = os.fstat(other.fd)
        require((current.st_dev, current.st_ino, current.st_mode, current.st_uid, current.st_gid) ==
                (self.before.st_dev, self.before.st_ino, self.before.st_mode, self.before.st_uid, self.before.st_gid))


def opened(path, stack, mode=None, executable=False, limit=MAX_CIPHER):
    absolute(path)
    parent, name = os.path.split(path)
    directory = Directory(parent, stack)
    fd = os.open(name, FILE_FLAGS, dir_fd=directory.fd)
    stack.callback(os.close, fd)
    before = os.fstat(fd)
    require(stat.S_ISREG(before.st_mode) and before.st_nlink == 1 and before.st_uid == 0)
    require(not stat.S_IMODE(before.st_mode) & 0o7022 and 0 < before.st_size <= limit)
    require(mode is None or stat.S_IMODE(before.st_mode) == mode)
    require(not executable or before.st_mode & stat.S_IXUSR)
    result = (fd, before, directory, name)
    bound_file(result, stack)
    return result


def bound_file(binding, stack):
    fd, before, directory, name = binding
    directory.bound(stack)
    require(fingerprint(before) == fingerprint(os.fstat(fd)) ==
            fingerprint(os.stat(name, dir_fd=directory.fd, follow_symlinks=False)))


class Output:
    def __init__(self, path, stack):
        absolute(path)
        parent, self.name = os.path.split(path)
        self.directory = Directory(parent, stack, private=True)
        try:
            os.stat(self.name, dir_fd=self.directory.fd, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise ValueError("output_exists")
        self.temporary = ".portable-factor-partial-" + secrets.token_hex(16)
        self.fd = os.open(self.temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                          0o600, dir_fd=self.directory.fd)
        os.fchmod(self.fd, 0o600)
        stack.callback(os.close, self.fd)
        stack.callback(self.cleanup)

    def cleanup(self):
        try:
            item = os.stat(self.temporary, dir_fd=self.directory.fd, follow_symlinks=False)
            held = os.fstat(self.fd)
            if (item.st_dev, item.st_ino) == (held.st_dev, held.st_ino):
                os.unlink(self.temporary, dir_fd=self.directory.fd)
                os.fsync(self.directory.fd)
        except OSError:
            pass  # 只保留本次 partial，不改其他名字或既有目标。

    def publish(self, stack):
        self.directory.bound(stack)
        item = os.fstat(self.fd)
        require(stat.S_ISREG(item.st_mode) and item.st_uid == 0 and item.st_nlink == 1)
        require(stat.S_IMODE(item.st_mode) == 0o600 and 64 <= item.st_size <= MAX_CIPHER)
        require(fingerprint(item) == fingerprint(os.stat(self.temporary, dir_fd=self.directory.fd, follow_symlinks=False)))
        os.fsync(self.fd)
        os.link(self.temporary, self.name, src_dir_fd=self.directory.fd, dst_dir_fd=self.directory.fd,
                follow_symlinks=False)
        os.fsync(self.directory.fd)
        os.unlink(self.temporary, dir_fd=self.directory.fd)
        os.fsync(self.directory.fd)


def host_key(path, output, stack, importing):
    absolute(path)
    require(path != output.directory.path + "/" + output.name)
    if importing:
        require(path.startswith(output.directory.path + "/"))
        parent, leaf = os.path.split(path)
        directory = Directory(parent, stack, private=True)
        try:
            os.stat(leaf, dir_fd=directory.fd, follow_symlinks=False)
        except FileNotFoundError:
            return None
    return opened(path, stack, mode=0o400, limit=16384)
