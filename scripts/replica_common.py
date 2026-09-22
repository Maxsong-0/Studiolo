"""副本协议及不跟随链接的文件系统边界（仅标准库）。"""
import hashlib
import json
import os
import re
import stat
import unicodedata

ALLOW = sorted([
    # 不含 项目/Semiment-Docs：该项目 199 MB（产品规格书等附件），单独就超过 MAX_TOTAL(128 MB)。
    # 其余 22 个项目 ＋ _规则 合计约 10 MB。要纳入它需先分离附件或提高 MAX_TOTAL，另行拍板。
    "_规则",
    "项目/AI Quota Flow",
    "项目/AI 网关门户",
    "项目/Brief 简报站",
    "项目/Codex源码分析",
    "项目/DocReader",
    "项目/Echo",
    "项目/HR系统",
    "项目/MindGarden",
    "项目/NAS 网关",
    "项目/Perplexity",
    "项目/Semiment",
    "项目/Studiolo",
    "项目/Tillerwright",
    "项目/brian-web",
    "项目/hermes",
    "项目/max-personal-web",
    "项目/培训 App",
    "项目/盖章",
    "项目/研究项目",
    "项目/绩效",
    "项目/远程桌面网关",
    "项目/销售报表",
])
MAX_ENTRIES = 25_000
MAX_MANIFEST = 4 * 1024 * 1024
MAX_FILE = 16 * 1024 * 1024
MAX_TOTAL = 128 * 1024 * 1024
MAX_NOTES_TOTAL = 16 * 1024 * 1024
DIR_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK


class Invalid(ValueError):
    pass


def require(ok):
    if not ok:
        raise Invalid("validation_failed")


def path_ok(path):
    require(isinstance(path, str) and 0 < len(path.encode("utf-8")) <= 4102)
    parts = path.split("/")
    require(len(parts) <= 65)
    require(all(p and not p.startswith(".") and len(p.encode("utf-8")) <= 255 for p in parts))
    require("\\" not in path and unicodedata.normalize("NFC", path) == path)
    require(not re.match(r"^[a-zA-Z]:/", path))
    require(not any(v in path.lower() for v in ("%2f", "%5c", "%00", "%2e")))
    for c in path:
        n = ord(c)
        require(unicodedata.category(c) != "Cc" and not any(
            lo <= n <= hi for lo, hi in ((0x300, 0x36f), (0x1ab0, 0x1aff),
                                       (0x1dc0, 0x1dff), (0x20d0, 0x20ff), (0xfe20, 0xfe2f))))
    return parts


def logical_path_ok(path):
    parts = path_ok(path)
    require(len(parts) <= 64 and len(path.encode("utf-8")) <= 4096)
    return parts


def load_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            require(key not in result)
            result[key] = value
        return result
    return json.loads(raw.decode("utf-8"), object_pairs_hook=pairs,
                      parse_constant=lambda _: require(False))


def encode(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def integer(value):
    return type(value) is int


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def fingerprint(st):
    return (st.st_dev, st.st_ino, st.st_mode, st.st_uid, st.st_gid, st.st_nlink,
            st.st_size, st.st_mtime_ns, st.st_ctime_ns)


def check_node(st, directory, private=False, ancestor=False):
    require(stat.S_ISDIR(st.st_mode) if directory else stat.S_ISREG(st.st_mode))
    require(st.st_uid in ({0, os.getuid()} if ancestor else {os.getuid()}))
    mode = stat.S_IMODE(st.st_mode)
    if private:
        require(mode == (0o700 if directory else 0o600))
    else:
        require(not mode & 0o022 or (ancestor and st.st_uid == 0 and mode & stat.S_ISVTX))
    if not directory:
        require(st.st_nlink == 1)


def open_root(path, create=False, private=False):
    """逐个打开绝对路径分量；现有祖先不可通过 resolve() 掩盖链接。"""
    require(os.path.isabs(path) and path != "/")
    parts = path.split("/")[1:]
    require(all(p not in ("", ".", "..") for p in parts))
    fd = os.open("/", DIR_FLAGS)
    try:
        for index, part in enumerate(parts):
            last = index == len(parts) - 1
            try:
                child = os.open(part, DIR_FLAGS, dir_fd=fd)
            except FileNotFoundError:
                require(create)
                os.mkdir(part, 0o700, dir_fd=fd)
                child = os.open(part, DIR_FLAGS, dir_fd=fd)
            try:
                check_node(os.fstat(child), True, private=private and last, ancestor=not last)
            except BaseException:
                os.close(child)
                raise
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def open_dir(parent, name, create=False, private=True):
    path_ok(name)
    require("/" not in name)
    if create:
        try:
            os.mkdir(name, 0o700, dir_fd=parent)
            os.fsync(parent)
        except FileExistsError:
            pass
    fd = os.open(name, DIR_FLAGS, dir_fd=parent)
    try:
        check_node(os.fstat(fd), True, private=private)
        require(os.fstat(fd).st_dev == os.fstat(parent).st_dev)
        return fd
    except BaseException:
        os.close(fd)
        raise


def descend(parent, path, create=False, private=True):
    fd = os.dup(parent)
    try:
        for part in path_ok(path):
            child = open_dir(fd, part, create, private)
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def read_file(parent, name, limit, private=True):
    fd = os.open(name, FILE_FLAGS, dir_fd=parent)
    try:
        before = os.fstat(fd)
        check_node(before, False, private)
        require(before.st_dev == os.fstat(parent).st_dev)
        require(before.st_size <= limit)
        data = bytearray()
        while True:
            chunk = os.read(fd, min(65536, limit + 1 - len(data)))
            if not chunk:
                break
            data.extend(chunk)
            require(len(data) <= limit)
        require(fingerprint(before) == fingerprint(os.fstat(fd)))
        require(fingerprint(before) == fingerprint(os.stat(name, dir_fd=parent, follow_symlinks=False)))
        require(len(data) == before.st_size)
        return bytes(data), fingerprint(before)
    finally:
        os.close(fd)


def write_file(parent, name, raw):
    fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=parent)
    try:
        with os.fdopen(fd, "wb", closefd=False) as out:
            out.write(raw)
            out.flush()
            os.fsync(fd)
    finally:
        os.close(fd)


def manifest(raw):
    require(len(raw) <= MAX_MANIFEST)
    value = load_json(raw)
    require(isinstance(value, dict) and set(value) == {"schema_version", "allow", "entries"})
    require(type(value["schema_version"]) is int and value["schema_version"] == 1 and value["allow"] == ALLOW)
    entries = value["entries"]
    require(isinstance(entries, list) and len(entries) <= MAX_ENTRIES)
    expected = {"vault": ("dir", 0, None)}
    for allow in ALLOW:
        pieces = path_ok(allow)
        for n in range(1, len(pieces) + 1):
            expected["vault/" + "/".join(pieces[:n])] = ("dir", 0, None)
    seen = {p.lower() for p in expected}
    previous, total, notes_total = "", 0, 0
    for entry in entries:
        require(isinstance(entry, dict) and set(entry) == {"path", "kind", "bytes", "sha256"})
        p, kind, size, sha = (entry[k] for k in ("path", "kind", "bytes", "sha256"))
        logical_path_ok(p)
        require(p > previous and any(p.startswith(a + "/") for a in ALLOW))
        require("vault/" + p.lower() not in seen)
        seen.add("vault/" + p.lower())
        require(integer(size) and 0 <= size <= (MAX_MANIFEST if p.lower().endswith(".md") else MAX_FILE))
        require(kind in ("dir", "file"))
        if kind == "dir":
            require(size == 0 and sha is None)
        else:
            require(isinstance(sha, str) and re.fullmatch("[a-f0-9]{64}", sha))
        expected["vault/" + p] = (kind, size, sha)
        previous, total = p, total + size
        if p.lower().endswith(".md"):
            notes_total += size
    require(total <= MAX_TOTAL)
    require(notes_total <= MAX_NOTES_TOTAL)
    for p in expected:
        if "/" in p:
            require(expected.get(p.rsplit("/", 1)[0], (None,))[0] == "dir")
    return value, expected
