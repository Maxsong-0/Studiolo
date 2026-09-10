"""客户端投影专属目录：逐层拒绝链接、验证绑定并保留用户改动。"""
import os
import re
import stat

from replica_cleanup import names_bounded
from replica_common import (DIR_FLAGS, check_node, encode, fingerprint, open_root,
                            read_file, require, write_file)

MARKER = encode({"schema_version": 1, "kind": "studiolo-client-projection"})
ID = re.compile(r"[a-f0-9]{64}")


def identity(fd):
    st = os.fstat(fd)
    return st.st_dev, st.st_ino


def read_root(path):
    """只读逐层打开；缺失分量保留 FileNotFoundError，已有祖先仍全部验证。"""
    require(isinstance(path, str) and os.path.isabs(path) and path != "/")
    parts = path.split("/")[1:]
    require(all(p not in ("", ".", "..") for p in parts))
    fd = os.open("/", DIR_FLAGS)
    try:
        for index, part in enumerate(parts):
            child = os.open(part, DIR_FLAGS, dir_fd=fd)
            try:
                last = index == len(parts) - 1
                check_node(os.fstat(child), True, private=last, ancestor=not last)
            except BaseException:
                os.close(child)
                raise
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def directory(parent, name, create=False):
    # 原生配置允许固定点目录；具体可写路径仍由 bundle schema 限定。
    require(name not in ("", ".", "..") and "/" not in name and "\\" not in name)
    if create:
        try:
            os.mkdir(name, 0o700, dir_fd=parent)
            os.fsync(parent)
        except FileExistsError:
            pass
    fd = os.open(name, DIR_FLAGS, dir_fd=parent)
    try:
        check_node(os.fstat(fd), True, private=True)
        require(identity(fd)[0] == identity(parent)[0])
        return fd
    except BaseException:
        os.close(fd)
        raise


def bound_directory(parent, name, fd):
    current = directory(parent, name)
    try:
        require(identity(current) == identity(fd))
    finally:
        os.close(current)


def write_path(root, path, raw):
    parts = path.split("/")
    fd = os.dup(root)
    try:
        for part in parts[:-1]:
            child = directory(fd, part, create=True)
            os.close(fd)
            fd = child
        write_file(fd, parts[-1], raw)
        os.fsync(fd)
    finally:
        os.close(fd)


def pointer(root):
    try:
        st = os.stat("current", dir_fd=root, follow_symlinks=False)
    except FileNotFoundError:
        return None, None
    require(stat.S_ISLNK(st.st_mode) and st.st_uid == os.getuid() and st.st_nlink == 1)
    link = os.readlink("current", dir_fd=root)
    require(re.fullmatch(r"releases/[a-f0-9]{64}", link))
    require(fingerprint(st) == fingerprint(os.stat("current", dir_fd=root, follow_symlinks=False)))
    return link.split("/")[1], fingerprint(st)


class View:
    """只读打开或显式初始化专属根；持有 fd 以复核名称没有被重新绑定。"""

    def __init__(self, path, create=False):
        self.path = os.fspath(path)
        self.root = self.releases = self.lock = None
        try:
            self.root = (open_root(self.path, create=True, private=True)
                         if create else read_root(self.path))
            entries = set(names_bounded(self.root, 32))
            self.empty = not entries
            if entries:
                self.validate_layout()
        except BaseException:
            self.close()
            raise

    def validate_layout(self, temporary=None):
        entries = set(names_bounded(self.root, 32))
        allowed = {"store.json", "projection.lock", "releases", "current"}
        if temporary:
            allowed.add(temporary)
        require(entries <= allowed)
        require({"store.json", "projection.lock", "releases"} <= entries)
        require(read_file(self.root, "store.json", 1024)[0] == MARKER)
        require(read_file(self.root, "projection.lock", 0)[0] == b"")
        if self.releases is None:
            self.releases = directory(self.root, "releases")
        else:
            bound_directory(self.root, "releases", self.releases)
        for name in names_bounded(self.releases, 1024):
            require(ID.fullmatch(name))
            fd = directory(self.releases, name)
            os.close(fd)

    def initialize(self):
        # 调用者已持有锁；初始化只接受空目录和刚创建的那一个锁。
        if self.empty:
            require(set(names_bounded(self.root, 32)) == {"projection.lock"})
            write_file(self.root, "store.json", MARKER)
            self.releases = directory(self.root, "releases", create=True)
            self.empty = False
            os.fsync(self.root)

    def binding(self, temporary=None):
        fd = open_root(self.path, private=True)
        try:
            require(identity(fd) == identity(self.root))
        finally:
            os.close(fd)
        if self.lock is not None:
            st = os.stat("projection.lock", dir_fd=self.root, follow_symlinks=False)
            check_node(st, False, private=True)
            require((st.st_dev, st.st_ino) == identity(self.lock))
        if not self.empty:
            self.validate_layout(temporary)

    def close(self):
        for name in ("lock", "releases", "root"):
            fd = getattr(self, name, None)
            if fd is not None:
                os.close(fd)
                setattr(self, name, None)

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
