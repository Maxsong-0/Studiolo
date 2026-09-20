"""私有创建/校验流程；不执行包内代码，不停止服务。"""
import contextlib
import os
import re
import sqlite3
import time
import urllib.parse
from human_snapshot_io import (
    DB_SECONDS, MAX_FILE, MAX_TOTAL, Tree, absolute, decode, encoded, require, safe, scan,
)
from human_snapshot_format import (
    SEED_REF, binding_valid, cipher, config_binding, database,
    manifest_valid, release_manifest,
)

@contextlib.contextmanager
def connection(tree, name, immutable=True):
    parent, _, leaf = safe(name).rpartition("/")
    with tree.directory(parent) as fd:
        path = urllib.parse.quote(f"/proc/self/fd/{fd}/{leaf}", safe="/")
        conn = sqlite3.connect("file:" + path + "?mode=ro" + ("&immutable=1" if immutable else ""), uri=True, timeout=3)
        try:
            conn.execute("PRAGMA trusted_schema=OFF")
            conn.execute("PRAGMA query_only=ON")
            yield conn
        finally:
            conn.close()


def db_source(scanned, name):
    files, dirs, _ = scanned
    require(not dirs and name in files and set(files) <= {name, name + "-wal", name + "-shm"})
    require((name + "-wal" in files) == (name + "-shm" in files))
    require(all(v["mode"] == 0o600 for v in files.values()))


def snapshot(source, scanned, name, output, target, version):
    # 只读 SQLite 仍可能改变 SHM；只打开已逐字节校验的私有暂存副本。
    stage_name = "STAGING-" + str(version)
    output.mkdir(stage_name)
    for item in sorted(scanned[0]):
        output.copy(source, item, stage_name + "/" + item)
    with contextlib.ExitStack() as stack:
        stage = Tree(output.path + "/" + stage_name, stack)
        require(scan(stage)[0] == scanned[0])
        output.write(target, b"")
        parent, _, leaf = target.rpartition("/")
        with output.directory(parent) as fd:
            with connection(stage, name, immutable=name + "-wal" not in scanned[0]) as src:
                dst = sqlite3.connect(f"/proc/self/fd/{fd}/{leaf}", timeout=3)
                try:
                    deadline = time.monotonic() + DB_SECONDS
                    def progress(status, remaining, total):
                        require(time.monotonic() < deadline and total <= MAX_FILE // 512)
                    src.backup(dst, pages=256, progress=progress, sleep=0.01)
                    require(dst.execute("PRAGMA journal_mode=DELETE").fetchone() == ("delete",))
                    dst.execute("PRAGMA trusted_schema=OFF")
                    database(dst, version)
                finally:
                    dst.close()
        # 只删除本函数创建且已核对的暂存文件，不按路径递归清理。
        names = set(os.listdir(stage.fd))
        require(names <= {name, name + "-wal", name + "-shm", name + "-journal"})
        for item in names:
            with stage.file(item, private=True):
                pass
            os.unlink(item, dir_fd=stage.fd)
    os.rmdir(stage_name, dir_fd=output.fd)
    with output.file(target, private=True) as fd:
        os.fsync(fd)


def verify_tree(tree, incomplete=False):
    before = scan(tree)
    value = decode(tree.read("manifest.json", private=True))
    manifest_valid(value)
    actual = dict(before[0])
    require(actual.pop("manifest.json")["mode"] == 0o600)
    if incomplete:
        require(tree.read("INCOMPLETE", 64, private=True) == b"incomplete\n")
        actual.pop("INCOMPLETE")
    require(actual == value["files"] and before[1] == value["directories"])
    config_binding(tree.read("owner.json", private=True), value["source_binding"])
    cipher(tree, "portable-factor.age")
    with contextlib.ExitStack() as stack:
        release = Tree(tree.path + "/release", stack)
        require(release_manifest(release, scan(release)) == value["source_release_manifest_sha256"])
    for path, version in (("auth/auth.sqlite3", 4), ("audit/audit.sqlite3", 2)):
        with tree.file(path, private=True) as fd:
            header = os.read(fd, 100)
            require(len(header) == 100 and header[:16] == b"SQLite format 3\x00" and header[18:20] == b"\x01\x01")
        with connection(tree, path) as conn:
            require(conn.execute("PRAGMA journal_mode").fetchone() == ("delete",))
            database(conn, version)
    require(scan(tree) == before)
    return value


def verify(path):
    with contextlib.ExitStack() as stack:
        return verify_tree(Tree(path, stack))


def create(args, between_scans=None):
    require(type(args.snapshot_id) is str and re.fullmatch(r"[A-Za-z0-9-]{1,64}", args.snapshot_id))
    binding = {"auth_directory": args.auth_dir, "audit_directory": args.audit_dir,
               "config": args.config, "release": args.release, "portable_factor": args.portable_factor,
               "seed_ref": SEED_REF, "credential_id": "totp-next"}
    binding_valid(binding)
    absolute(args.output)
    require(all(args.output != p and not args.output.startswith(p + "/")
                and not p.startswith(args.output + "/")
                for p in (args.auth_dir, args.audit_dir, args.release, args.config, args.portable_factor)))
    with contextlib.ExitStack() as stack:
        output = Tree(args.output, stack, new=True)
        output.write("INCOMPLETE", b"incomplete\n")
        sources = [(Tree(path, stack), None) for path in (args.auth_dir, args.audit_dir, args.release)]
        for path in (args.config, args.portable_factor):
            parent, leaf = os.path.split(path)
            sources.append((Tree(parent, stack), leaf))
        before = [scan(tree, leaf) for tree, leaf in sources]
        require(sum(item["size"] for scanned in before for item in scanned[0].values()) <= MAX_TOTAL)
        db_source(before[0], "auth.sqlite3")
        db_source(before[1], "audit.sqlite3")
        release_sha = release_manifest(sources[2][0], before[2])
        config_binding(sources[3][0].read(sources[3][1], private=True), binding)
        cipher(*sources[4])
        for name in ("auth", "audit", "release"):
            output.mkdir(name)
        snapshot(sources[0][0], before[0], "auth.sqlite3", output, "auth/auth.sqlite3", 4)
        snapshot(sources[1][0], before[1], "audit.sqlite3", output, "audit/audit.sqlite3", 2)
        output.copy(sources[3][0], sources[3][1], "owner.json")
        output.copy(sources[4][0], sources[4][1], "portable-factor.age")
        for name in sorted(before[2][1], key=lambda p: (p.count("/"), p)):
            output.mkdir("release/" + name)
        for name, item in sorted(before[2][0].items()):
            mode = item["mode"] if name.startswith("bin/") else 0o600
            output.copy(sources[2][0], name, "release/" + name, mode)
        if between_scans:
            between_scans()
        require([scan(tree, leaf) for tree, leaf in sources] == before)
        files, directories, _ = scan(output)
        files.pop("INCOMPLETE")
        value = {"schema_version": 1, "state": "complete", "snapshot_id": args.snapshot_id,
                 "source_binding": binding, "source_release_manifest_sha256": release_sha,
                 "files": files, "directories": directories}
        manifest_valid(value)
        output.write("manifest.json", encoded(value))
        verify_tree(output, incomplete=True)
        require([scan(tree, leaf) for tree, leaf in sources] == before)
        os.fsync(output.fd)
        os.unlink("INCOMPLETE", dir_fd=output.fd)
        try:
            os.fsync(output.fd)
        except OSError:
            output.write("INCOMPLETE", b"incomplete\n")
            raise
    return value
