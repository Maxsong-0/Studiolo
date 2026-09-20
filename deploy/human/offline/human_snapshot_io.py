"""私有文件描述符边界及有界 JSON 工具。"""
import contextlib
import hashlib
import json
import os
import stat
import sys
import unicodedata

DIR = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
FILE = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC
MAX_NODES, MAX_FILE, MAX_TOTAL = 16384, 1024**3, 4 * 1024**3
MAX_JSON, MAX_CIPHER, DB_SECONDS = 4 * 1024**2, 64 * 1024, 15


def require(ok):
    if not ok:
        raise ValueError("validation_failed")


def safe(path):
    require(type(path) is str and 0 < len(path.encode("utf-8")) <= 4096)
    parts = path.split("/")
    keep = parts[-1] == ".gitkeep" and path.startswith(("example-vault/", "release/example-vault/"))
    require(len(parts) <= 64 and all(p and (not p.startswith(".") or (keep and i == len(parts) - 1))
                                    for i, p in enumerate(parts)))
    require("\\" not in path and unicodedata.normalize("NFC", path) == path)
    require(not any(unicodedata.category(c).startswith("C") for c in path))
    return path


def absolute(path):
    require(type(path) is str and path.startswith("/") and path != "/")
    require(len(path.encode("utf-8")) <= 4096 and len(path.split("/")) <= 65)
    require(all(part not in ("", ".", "..") for part in path.split("/")[1:]))
    require("\\" not in path and unicodedata.normalize("NFC", path) == path)
    require(not any(unicodedata.category(c).startswith("C") for c in path))
    return path


def fp(st):
    return (st.st_dev, st.st_ino, st.st_mode, st.st_uid, st.st_gid,
            st.st_nlink, st.st_size, st.st_mtime_ns, st.st_ctime_ns)


def mount_id(fd):
    # 只比较 st_dev 无法识别同一文件系统上的 bind mount。
    with open(f"/proc/self/fdinfo/{fd}", "r", encoding="ascii") as stream:
        rows = [line.split()[1] for line in stream if line.startswith("mnt_id:")]
    require(len(rows) == 1 and rows[0].isdigit())
    return int(rows[0])


def unique(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result)
        result[key] = value
    return result


def decode(raw):
    require(len(raw) <= MAX_JSON)
    return json.loads(raw, object_pairs_hook=unique, parse_constant=lambda _: require(False))


def encoded(value):
    return (json.dumps(value, ensure_ascii=False, sort_keys=True, indent=2) + "\n").encode("utf-8")


def parents(names):
    result = set()
    for name in names:
        safe(name)
        parts = name.split("/")
        result.update("/".join(parts[:i]) for i in range(1, len(parts)))
    return result


class Tree:
    """以禁止跟随链接的目录句柄固定边界；树内后代须处于同一挂载。"""
    def __init__(self, path, stack, new=False, private=True):
        require(sys.platform == "linux")
        absolute(path)
        allowed = {0, os.geteuid()} | ({1000} if os.geteuid() == 0 else set())
        source_owners = {os.geteuid()} | ({1000} if os.geteuid() == 0 else set())
        fd = os.open("/", DIR)
        stack.callback(os.close, fd)
        parts = path.split("/")[1:]
        for index, part in enumerate(parts):
            last = index == len(parts) - 1
            if new and last:
                os.mkdir(part, 0o700, dir_fd=fd)
            fd = os.open(part, DIR, dir_fd=fd)
            stack.callback(os.close, fd)
            st = os.fstat(fd)
            mode = stat.S_IMODE(st.st_mode)
            require(st.st_uid in allowed)
            require(not mode & 0o022 or (not last and st.st_uid == 0 and mode & stat.S_ISVTX))
            require(not mode & 0o6000)
        require(st.st_uid in source_owners)
        require(not private or stat.S_IMODE(st.st_mode) == 0o700)
        require(not new or st.st_uid == os.geteuid())
        self.fd, self.path, self.owner = fd, path, st.st_uid
        self.device, self.mount, self.stack = st.st_dev, mount_id(fd), stack

    def node(self, fd, directory, private=False):
        st = os.fstat(fd)
        require(stat.S_ISDIR(st.st_mode) if directory else stat.S_ISREG(st.st_mode))
        require(st.st_uid == self.owner and st.st_dev == self.device and mount_id(fd) == self.mount)
        mode = stat.S_IMODE(st.st_mode)
        require(not mode & 0o7022 and (directory or st.st_nlink == 1))
        if private:
            require(mode == (0o700 if directory else 0o600))
        return st

    @contextlib.contextmanager
    def directory(self, path=""):
        fd = os.dup(self.fd)
        try:
            if path:
                for name in safe(path).split("/"):
                    child = os.open(name, DIR, dir_fd=fd)
                    os.close(fd)
                    fd = child
                    self.node(fd, True)
            yield fd
        finally:
            os.close(fd)

    @contextlib.contextmanager
    def file(self, path, private=False):
        safe(path)
        parent, _, name = path.rpartition("/")
        with self.directory(parent) as base:
            before = os.stat(name, dir_fd=base, follow_symlinks=False)
            fd = os.open(name, FILE, dir_fd=base)
            try:
                require(fp(before) == fp(self.node(fd, False, private)))
                yield fd
                require(fp(before) == fp(os.fstat(fd)) == fp(os.stat(name, dir_fd=base, follow_symlinks=False)))
            finally:
                os.close(fd)

    def read(self, path, limit=MAX_JSON, private=False):
        with self.file(path, private) as fd:
            st = os.fstat(fd)
            require(st.st_size <= limit)
            with os.fdopen(os.dup(fd), "rb") as stream:
                raw = stream.read(limit + 1)
            require(len(raw) == st.st_size)
            return raw

    def mkdir(self, path):
        parent, _, name = safe(path).rpartition("/")
        with self.directory(parent) as fd:
            os.mkdir(name, 0o700, dir_fd=fd)
            os.fsync(fd)

    def write(self, path, raw, mode=0o600):
        parent, _, name = safe(path).rpartition("/")
        with self.directory(parent) as fd:
            out = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, mode, dir_fd=fd)
            with os.fdopen(out, "wb") as stream:
                os.fchmod(out, mode)
                stream.write(raw)
                stream.flush()
                os.fsync(out)
            os.fsync(fd)

    def copy(self, source, name, target, mode=0o600):
        with source.file(name) as src:
            st = os.fstat(src)
            require(st.st_size <= MAX_FILE)
            parent, _, leaf = safe(target).rpartition("/")
            with self.directory(parent) as fd:
                out = os.open(leaf, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, mode, dir_fd=fd)
                with os.fdopen(out, "wb") as stream:
                    os.fchmod(out, mode)
                    size = 0
                    while chunk := os.read(src, 1024 * 1024):
                        size += len(chunk)
                        require(size <= st.st_size)
                        stream.write(chunk)
                    require(size == st.st_size)
                    stream.flush()
                    os.fsync(out)
                os.fsync(fd)

    def bound(self):
        with contextlib.ExitStack() as stack:
            other = Tree(self.path, stack, private=False)
            require(fp(os.fstat(self.fd)) == fp(os.fstat(other.fd)) and self.mount == other.mount)


def scan(tree, leaf=None):
    files, directories, fingerprints = {}, {}, {}
    total = 0

    def walk(base, relative):
        nonlocal total
        before = tree.node(base, True)
        fingerprints[relative] = fp(before)
        names = [leaf] if leaf is not None and not relative else sorted(os.listdir(base))
        for name in names:
            path = safe(relative + "/" + name if relative else name)
            require(len(fingerprints) < MAX_NODES)
            st = os.stat(name, dir_fd=base, follow_symlinks=False)
            directory = stat.S_ISDIR(st.st_mode)
            require(not directory or name != ".gitkeep")
            require(leaf is None or not directory)
            child = os.open(name, DIR if directory else FILE, dir_fd=base)
            try:
                require(fp(st) == fp(tree.node(child, directory)))
                fingerprints[path] = fp(st)
                if directory:
                    directories[path] = {"mode": stat.S_IMODE(st.st_mode)}
                    walk(child, path)
                else:
                    require(st.st_size <= MAX_FILE)
                    total += st.st_size
                    require(total <= MAX_TOTAL)
                    digest, size = hashlib.sha256(), 0
                    while chunk := os.read(child, 1024 * 1024):
                        size += len(chunk)
                        require(size <= st.st_size)
                        digest.update(chunk)
                    require(size == st.st_size)
                    files[path] = {"size": size, "sha256": digest.hexdigest(), "mode": stat.S_IMODE(st.st_mode)}
                require(fp(st) == fp(os.fstat(child)) == fp(os.stat(name, dir_fd=base, follow_symlinks=False)))
            finally:
                os.close(child)
        require(fp(before) == fp(os.fstat(base)))

    walk(tree.fd, "")
    tree.bound()
    return files, directories, fingerprints
