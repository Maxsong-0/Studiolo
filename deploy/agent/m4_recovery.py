#!/usr/bin/env python3
"""Linux M4 恢复包：调用者必须先停止全部写者，跨库一致性不由 backup API 提供。

处理既有身份库、v2 审计库、token、二进制及单元，可选 v1 策略库；不启停服务。
失败保留 INCOMPLETE 和已生成文件；manifest 的摘要不构成抵抗包所有者的数字签名。
"""
import argparse
import contextlib
import datetime
import hashlib
import json
import os
import re
import sqlite3
import stat
import sys
import urllib.parse

DIR_FLAGS = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
FILE_FLAGS = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
TOKEN = re.compile(r"[a-z0-9-]+\.token\Z")
VERSIONS = {"data/agent.sqlite3": 0, "data/audit/audit.sqlite3": 2}
POLICY = {"policy/policy.sqlite3": 1}
FIXED = set(VERSIONS) | {"bin/studiolo-agent", "previous.service"}
DIRECTORIES = {"data", "data/audit", "config", "bin"}


def require(ok):
    if not ok:
        raise ValueError("恢复包校验失败（不回显数据或凭据）")


def mode_for(name):
    return 0o700 if name == "bin/studiolo-agent" else 0o600


def fingerprint(st):
    return (st.st_dev, st.st_ino, st.st_mode, st.st_uid, st.st_gid,
            st.st_nlink, st.st_size, st.st_mtime_ns, st.st_ctime_ns)


class Tree:
    """以不跟随链接的目录句柄固定边界；源只接受当前用户或 root 部署时的 uid 1000。"""
    def __init__(self, path, stack, new=False, owner=None, private=True):
        require(sys.platform == "linux" and os.path.isabs(path) and path != "/")
        parts = path.split("/")[1:]
        require(all(p not in ("", ".", "..") for p in parts))
        allowed = {0, os.geteuid()} | ({1000} if os.geteuid() == 0 else set())
        fd = os.open("/", DIR_FLAGS)
        stack.callback(os.close, fd)
        for index, part in enumerate(parts):
            last = index == len(parts) - 1
            if last and new:
                os.mkdir(part, 0o700, dir_fd=fd)
            fd = os.open(part, DIR_FLAGS, dir_fd=fd)
            stack.callback(os.close, fd)
            st = os.fstat(fd)
            require(st.st_uid in allowed)
            mode = stat.S_IMODE(st.st_mode)
            require(not mode & 0o022 or (not last and st.st_uid == 0 and mode & stat.S_ISVTX))
            if last:
                require(not private or mode == 0o700)
                require(owner is None or st.st_uid == owner)
        self.fd, self.owner, self.stack = fd, os.fstat(fd).st_uid, stack
        self.dirs = {"": fd}

    def directory(self, path, new=False):
        if path not in self.dirs:
            parent, _, name = path.rpartition("/")
            base = self.directory(parent)
            if new:
                os.mkdir(name, 0o700, dir_fd=base)
            fd = os.open(name, DIR_FLAGS, dir_fd=base)
            self.stack.callback(os.close, fd)
            st = os.fstat(fd)
            require(st.st_uid == self.owner and stat.S_IMODE(st.st_mode) == 0o700)
            self.dirs[path] = fd
        return self.dirs[path]

    def file(self, path, new=False, private=True):
        parent, _, name = path.rpartition("/")
        base = self.directory(parent)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW if new else FILE_FLAGS
        fd = os.open(name, flags, mode_for(path), dir_fd=base)
        self.stack.callback(os.close, fd)
        st = os.fstat(fd)
        require(stat.S_ISREG(st.st_mode) and st.st_nlink == 1 and st.st_uid == self.owner)
        require(stat.S_IMODE(st.st_mode) == mode_for(path) if private else not st.st_mode & 0o022)
        return fd

    def bound(self, path, fd, before):
        parent, _, name = path.rpartition("/")
        require(fingerprint(before) == fingerprint(os.fstat(fd)) == fingerprint(
            os.stat(name, dir_fd=self.directory(parent), follow_symlinks=False)))

    def write(self, path, raw):
        fd = self.file(path, new=True)
        with os.fdopen(os.dup(fd), "wb") as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(fd)

    def copy(self, source, name, target=None, private=True):
        fd = source.file(name, private=private)
        before = os.fstat(fd)
        dest = self.file(target or name, new=True)
        with os.fdopen(os.dup(dest), "wb") as stream:
            while chunk := os.read(fd, 1024 * 1024):
                stream.write(chunk)
            stream.flush()
            os.fsync(dest)
        source.bound(name, fd, before)

    def entries(self, policy=False):
        files, directories = set(), set()
        expected = DIRECTORIES | ({"policy"} if policy else set())
        def walk(path):
            fd = self.directory(path)
            for name in os.listdir(fd):
                entry = path + "/" + name if path else name
                st = os.stat(name, dir_fd=fd, follow_symlinks=False)
                if stat.S_ISDIR(st.st_mode):
                    require(entry in expected)
                    directories.add(entry)
                    walk(entry)
                else:
                    require(stat.S_ISREG(st.st_mode))
                    files.add(entry)
        walk("")
        require(directories == expected)
        return files


def connection(tree, name, immutable=False):
    parent, _, leaf = name.rpartition("/")
    uri = "file:" + urllib.parse.quote(f"/proc/self/fd/{tree.directory(parent)}/{leaf}")
    conn = sqlite3.connect(uri + "?mode=ro" + ("&immutable=1" if immutable else ""), uri=True, timeout=3)
    conn.execute("PRAGMA trusted_schema=OFF")
    conn.execute("PRAGMA query_only=ON")
    return contextlib.closing(conn)


def check_database(conn, version):
    require(conn.execute("PRAGMA user_version").fetchone() == (version,))
    require(conn.execute("PRAGMA quick_check(1)").fetchall() == [("ok",)])
    require(conn.execute("PRAGMA foreign_key_check").fetchone() is None)


def snapshot(source, name, output, target, version):
    bindings = []
    parent, _, leaf = name.rpartition("/")
    names = set(os.listdir(source.directory(parent)))
    require(leaf + "-journal" not in names)
    wal = leaf + "-wal" in names
    require(not wal or leaf + "-shm" in names)
    for item in [name] + [name + suffix for suffix in ("-wal", "-shm") if leaf + suffix in names]:
        fd = source.file(item)
        bindings.append((item, fd, os.fstat(fd)))
    output.file(target, new=True)
    dest_parent, _, dest_name = target.rpartition("/")
    dest_path = f"/proc/self/fd/{output.directory(dest_parent)}/{dest_name}"
    # 无 WAL 时 immutable 防止只读连接在源目录新建 root 所有的 WAL/SHM。
    with connection(source, name, immutable=not wal) as src, contextlib.closing(sqlite3.connect(dest_path)) as dst:
        src.backup(dst)
        require(dst.execute("PRAGMA journal_mode=DELETE").fetchone() == ("delete",))
        check_database(dst, version)
    for item, fd, before in bindings:
        # 只读 WAL 读者仍可更新共享内存里的读锁；SHM 内容不是恢复数据。
        if item.endswith("-shm"):
            require(fingerprint(before)[:6] == fingerprint(os.fstat(fd))[:6])
            before = os.fstat(fd)
        source.bound(item, fd, before)
    fd = output.file(target)
    os.fsync(fd)


def metadata(tree, names):
    result = {}
    for name in sorted(names):
        fd = tree.file(name)
        before, digest = os.fstat(fd), hashlib.sha256()
        while chunk := os.read(fd, 1024 * 1024):
            digest.update(chunk)
        tree.bound(name, fd, before)
        result[name] = {"sha256": digest.hexdigest(), "size": before.st_size}
    return result


def verify(tree, incomplete=False):
    def unique(pairs):
        value = {}
        for key, item in pairs:
            require(key not in value)
            value[key] = item
        return value
    fd = tree.file("manifest.json")
    before = os.fstat(fd)
    require(before.st_size <= 1024 * 1024)
    with os.fdopen(os.dup(fd), "rb") as stream:
        value = json.load(stream, object_pairs_hook=unique, parse_constant=lambda _: require(False))
    tree.bound("manifest.json", fd, before)
    require(type(value) is dict and set(value) == {"schema_version", "state", "created_at", "database_versions", "files"})
    require(type(value["schema_version"]) is int and value["schema_version"] == 1 and value["state"] == "complete")
    require(type(value["created_at"]) is str and re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", value["created_at"]))
    datetime.datetime.strptime(value["created_at"], "%Y-%m-%dT%H:%M:%SZ")
    versions = value["database_versions"]
    require(versions in (VERSIONS, VERSIONS | POLICY) and all(type(v) is int for v in versions.values()))
    fixed = FIXED | (set(POLICY) if versions == VERSIONS | POLICY else set())
    files = value["files"]
    require(type(files) is dict and len(fixed) < len(files) <= len(fixed) + 64 and fixed <= set(files))
    require(all(name.startswith("config/") and TOKEN.fullmatch(name[7:]) for name in set(files) - fixed))
    for item in files.values():
        require(type(item) is dict and set(item) == {"sha256", "size"})
        require(type(item["size"]) is int and item["size"] >= 0)
        require(type(item["sha256"]) is str and re.fullmatch("[a-f0-9]{64}", item["sha256"]))
    require(tree.entries(bool(set(POLICY) & set(versions))) == set(files) | {"manifest.json"} | ({"INCOMPLETE"} if incomplete else set()))
    require(metadata(tree, files) == files)
    for name, version in versions.items():
        with connection(tree, name, immutable=True) as conn:
            check_database(conn, version)
    return value


def populate(args, output, stack):
    data, config = Tree(args.data, stack), Tree(args.config, stack)
    require(data.owner == config.owner)
    names = set(os.listdir(config.fd))
    require(0 < len(names) <= 64 and all(TOKEN.fullmatch(name) for name in names))
    versions = VERSIONS | (POLICY if args.policy else {})
    for name, version in VERSIONS.items():
        snapshot(data, name[5:], output, name, version)
    if args.policy:
        snapshot(Tree(args.policy, stack, owner=data.owner), "policy.sqlite3", output, "policy/policy.sqlite3", 1)
    for name in sorted(names):
        output.copy(config, name, "config/" + name)
    for path, target in [(args.binary, "bin/studiolo-agent"), (args.unit, "previous.service")]:
        parent, leaf = os.path.split(path)
        source = Tree(parent, stack, private=False)
        require(source.owner in {0, data.owner})
        output.copy(source, leaf, target, private=False)
    return {"schema_version": 1, "state": "complete", "database_versions": versions,
            "created_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
            "files": metadata(output, FIXED | set(versions) | {"config/" + name for name in names})}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("create", help="外层先停全部写者，再创建跨库恢复点")
    for name in ("data", "config", "binary", "unit", "output"):
        create.add_argument("--" + name, required=True)
    create.add_argument("--policy")
    commands.add_parser("verify").add_argument("bundle")
    restore = commands.add_parser("restore-copy")
    restore.add_argument("bundle")
    restore.add_argument("output")
    args = parser.parse_args(argv)
    os.umask(0o077)
    with contextlib.ExitStack() as stack:
        if args.command != "create":
            source = Tree(args.bundle, stack, owner=os.geteuid())
            manifest = verify(source)
            if args.command == "verify":
                print("恢复包完整性验证通过。")
                return
        protected = [args.data, args.config] + ([args.policy] if args.policy else []) if args.command == "create" else [args.bundle]
        require(all(not os.path.normpath(args.output).startswith(os.path.normpath(path) + "/") for path in protected))
        output = Tree(args.output, stack, new=True, owner=os.geteuid())
        output.write("INCOMPLETE", b"incomplete\n")
        policy = bool(args.policy) if args.command == "create" else "policy/policy.sqlite3" in manifest["files"]
        for name in sorted(DIRECTORIES | ({"policy"} if policy else set())):
            output.directory(name, new=True)
        if args.command == "create":
            manifest = populate(args, output, stack)
        else:
            for name in manifest["files"]:
                output.copy(source, name)
        output.write("manifest.json", (json.dumps(manifest, sort_keys=True, indent=2) + "\n").encode())
        verify(output, incomplete=True)
        for fd in output.dirs.values():
            os.fsync(fd)
        os.unlink("INCOMPLETE", dir_fd=output.fd)
        try:
            os.fsync(output.fd)
        except OSError:
            output.write("INCOMPLETE", b"incomplete\n")
            raise
    print("恢复包已创建并验证。" if args.command == "create" else "隔离恢复副本已创建并验证。")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, sqlite3.Error):
        print("操作失败；已创建的不完整目录保留供检查，未覆盖既有路径。", file=sys.stderr)
        sys.exit(1)
