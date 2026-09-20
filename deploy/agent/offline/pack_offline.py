#!/usr/bin/env python3
"""Linux 离线封装：只读冻结输入；不加密、不传输、不使身份失效。"""
import argparse
import contextlib
import datetime
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import unicodedata

DIR = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
FILE = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
REQUIRED_TOOLS = {"m4_recovery.py", "replica_common.py", "replica_store.py", "replica_cleanup.py"}
MAX_NODES, MAX_BYTES = 100_000, 8 * 1024**3
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pack_offline_helpers import REPLICA_CHECK, require, safe, fp, mode, load, human_id

class Tree:
    def __init__(self, path, stack, new=False, private=True):
        require(sys.platform == "linux" and os.path.isabs(path) and path != "/")
        parts = path.split("/")[1:]
        require(all(p not in ("", ".", "..") for p in parts))
        allowed = {0, os.geteuid()} | ({1000} if os.geteuid() == 0 else set())
        fd = os.open("/", DIR)
        stack.callback(os.close, fd)
        for index, part in enumerate(parts):
            last = index == len(parts) - 1
            if new and last:
                os.mkdir(part, 0o700, dir_fd=fd)
            fd = os.open(part, DIR, dir_fd=fd)
            stack.callback(os.close, fd)
            st = os.fstat(fd)
            require(st.st_uid in allowed)
            require(not st.st_mode & 0o022 or (not last and st.st_uid == 0 and st.st_mode & stat.S_ISVTX))
        require(not private or stat.S_IMODE(st.st_mode) == 0o700)
        require(not new or st.st_uid == os.geteuid())
        self.fd, self.path, self.owner, self.device = fd, path, st.st_uid, st.st_dev

    def node(self, st, directory):
        require(stat.S_ISDIR(st.st_mode) if directory else stat.S_ISREG(st.st_mode))
        require(st.st_uid == self.owner and st.st_dev == self.device and not st.st_mode & 0o022)
        require(directory or st.st_nlink == 1)

    def bound(self):
        with contextlib.ExitStack() as stack:
            other = Tree(self.path, stack, private=False)
            require(fp(os.fstat(self.fd)) == fp(os.fstat(other.fd)))

def write(fd, name, raw, permissions=0o600):
    out = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, permissions, dir_fd=fd)
    with os.fdopen(out, "wb") as stream:
        stream.write(raw)
        stream.flush()
        os.fsync(stream.fileno())

def scan(tree, prefix="", output=None, leaf=None, strict=False, ignore=(), as_name=None):
    files, directories, snapshots, total = {}, [], {}, 0
    def walk(fd, relative, dest):
        nonlocal total
        before = os.fstat(fd)
        tree.node(before, True)
        if strict:
            require(stat.S_IMODE(before.st_mode) == 0o700)
        if leaf is None or relative:
            snapshots[relative] = fp(before)
        names = [leaf] if leaf is not None and not relative else sorted(os.listdir(fd))
        for name in names:
            if not relative and name in ignore:
                continue
            path = safe(relative + "/" + name if relative else name)
            destination = as_name if leaf is not None and not relative else name
            target = prefix + "/" + (as_name or path) if prefix else path
            require(len(snapshots) < MAX_NODES)
            st = os.stat(name, dir_fd=fd, follow_symlinks=False)
            directory = stat.S_ISDIR(st.st_mode)
            require(leaf is None or not directory)
            tree.node(st, directory)
            child = os.open(name, DIR if directory else FILE, dir_fd=fd)
            try:
                require(fp(st) == fp(os.fstat(child)))
                snapshots[path] = fp(st)
                if directory:
                    directories.append(target)
                    if dest is not None:
                        os.mkdir(name, 0o700, dir_fd=dest)
                    dest_child = os.open(name, DIR, dir_fd=dest) if dest is not None else None
                    try:
                        walk(child, path, dest_child)
                        if dest_child is not None:
                            os.fsync(dest_child)
                    finally:
                        if dest_child is not None:
                            os.close(dest_child)
                else:
                    require(st.st_size <= 1024**3)
                    total += st.st_size
                    require(total <= MAX_BYTES)
                    permissions = mode(target, stat.S_IMODE(st.st_mode))
                    if strict:
                        require(stat.S_IMODE(st.st_mode) == permissions)
                    digest, size = hashlib.sha256(), 0
                    out = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                  permissions, dir_fd=dest) if dest is not None else None
                    if out is not None:
                        os.fchmod(out, permissions)
                    with contextlib.ExitStack() as stack:
                        stream = stack.enter_context(os.fdopen(out, "wb")) if out is not None else None
                        while chunk := os.read(child, 1024 * 1024):
                            size += len(chunk)
                            require(size <= st.st_size)
                            digest.update(chunk)
                            if stream:
                                stream.write(chunk)
                        require(size == st.st_size)
                        if stream:
                            stream.flush()
                            os.fsync(stream.fileno())
                    files[target] = {"sha256": digest.hexdigest(), "size": size, "mode": permissions}
                require(fp(st) == fp(os.fstat(child)) == fp(os.stat(name, dir_fd=fd, follow_symlinks=False)))
            finally:
                os.close(child)
        if leaf is None or relative:
            require(fp(before) == fp(os.fstat(fd)))
    walk(tree.fd, "", output)
    tree.bound()
    return files, sorted(directories), snapshots

def validate(tools, recovery, replica=None, human=None):
    require(tools.owner == os.geteuid())
    require(REQUIRED_TOOLS <= set(os.listdir(tools.fd)))
    script = os.open("m4_recovery.py", FILE, dir_fd=tools.fd)
    try:
        tools.node(os.fstat(script), False)
        commands = [[sys.executable, "-I", "-B", f"/proc/self/fd/{script}", "verify", recovery]]
        if replica is not None:
            commands.append([sys.executable, "-I", "-B", "-c", REPLICA_CHECK,
                             f"/proc/self/fd/{tools.fd}", replica])
        for command in commands:
            result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                                    stderr=subprocess.DEVNULL, pass_fds=(script, tools.fd), timeout=120)
            require(result.returncode == 0)
    finally:
        os.close(script)
    if human is not None:
        require({"human_snapshot.py", "human_snapshot_io.py", "human_snapshot_format.py", "human_snapshot_core.py"} <= set(os.listdir(tools.fd)))
        # 工具目录来自外部固定来源；包内代码不获得执行权。
        script = os.open("human_snapshot.py", FILE, dir_fd=tools.fd)
        try:
            tools.node(os.fstat(script), False)
            result = subprocess.run([sys.executable, "-I", "-B", tools.path + "/human_snapshot.py", "verify", human],
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=120)
            require(result.returncode == 0)
        finally:
            os.close(script)


def verify(bundle, tools, incomplete=False):
    value = load(bundle)
    ignore = ("manifest.json", "INCOMPLETE") if incomplete else ("manifest.json",)
    actual = scan(bundle, strict=True, ignore=ignore)
    require((actual[0], {p: {"mode": 0o700} for p in actual[1]}) == (value["files"], value["directories"]))
    roots = {"recovery", "replica", "tools", "system"} | ({"human"} if value["schema_version"] == 2 else set())
    require({p.split("/")[0] for p in actual[1]} == roots)
    require(all("/" in p and p.split("/")[0] in roots for p in actual[0]))
    require({"tools/" + p for p in REQUIRED_TOOLS} <= set(actual[0]))
    require({p for p in actual[0] if p.startswith("system/")} == {"system/runtime-libraries.txt", "system/os-release"})
    require(not any(p.startswith("system/") for p in actual[1]))
    human = bundle.path + "/human" if value["schema_version"] == 2 else None
    validate(tools, bundle.path + "/recovery", bundle.path + "/replica", human)
    if human:
        with contextlib.ExitStack() as stack:
            require(human_id(Tree(human, stack)) == value["snapshot_id"])
    require(scan(bundle, strict=True, ignore=ignore) == actual)
    require(load(bundle) == value)

def pack(args, between_scans=None):
    with contextlib.ExitStack() as stack:
        names = ("recovery", "replica", "tools") + (("human",) if getattr(args, "human", None) else ())
        trees = [(name, Tree(getattr(args, name), stack), None) for name in names]
        for option, target in (("runtime_libraries", "runtime-libraries.txt"), ("os_release", "os-release")):
            parent, leaf = os.path.split(getattr(args, option))
            trees.append(("system/" + target, Tree(parent, stack, private=False), leaf))
        require(all(args.output != t.path and not args.output.startswith(t.path + "/") for _, t, leaf in trees if leaf is None))
        before = [scan(t, leaf=leaf, strict=name == "replica") for name, t, leaf in trees]
        tools = trees[2][1]
        validate(tools, args.recovery, human=getattr(args, "human", None))
        snapshot = human_id(trees[3][1]) if "human" in names else None
        output = Tree(args.output, stack, new=True)
        write(output.fd, "INCOMPLETE", b"incomplete\n")
        os.mkdir("system", 0o700, dir_fd=output.fd)
        for index, (prefix, tree, leaf) in enumerate(trees):
            if leaf is None:
                os.mkdir(prefix, 0o700, dir_fd=output.fd)
            directory = os.open(prefix if leaf is None else "system", DIR, dir_fd=output.fd)
            try:
                copied = scan(tree, prefix=prefix if leaf is None else "system", output=directory,
                              leaf=leaf, as_name=None if leaf is None else prefix.split("/")[1])
                require(copied[2] == before[index][2])
                os.fsync(directory)
            finally:
                os.close(directory)
        if between_scans:
            between_scans()
        require([scan(t, leaf=leaf) for _, t, leaf in trees] == before)
        files, directories, _ = scan(output, strict=True, ignore=("INCOMPLETE",))
        value = {"schema_version": 2 if snapshot else 1, "state": "complete", "created_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
                 "directories": {p: {"mode": 0o700} for p in directories}, "files": files}
        if snapshot:
            value["snapshot_id"] = snapshot
        write(output.fd, "manifest.json", (json.dumps(value, sort_keys=True, ensure_ascii=False, indent=2) + "\n").encode())
        verify(output, tools, incomplete=True)
        require([scan(t, leaf=leaf) for _, t, leaf in trees] == before)
        os.fsync(output.fd)
        os.unlink("INCOMPLETE", dir_fd=output.fd)
        try:
            os.fsync(output.fd)
        except OSError:
            write(output.fd, "INCOMPLETE", b"incomplete\n")
            raise

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("pack")
    for name in ("recovery", "replica", "tools", "runtime-libraries", "os-release", "output"):
        create.add_argument("--" + name, required=True)
    create.add_argument("--human", help="已冻结且独立验证的人侧快照；输出schema2")
    check = commands.add_parser("verify")
    check.add_argument("bundle")
    check.add_argument("--tools", required=True, help="独立可信工具目录，禁止盲信包内代码")
    args = parser.parse_args()
    os.umask(0o077)
    if args.command == "pack":
        pack(args)
    else:
        with contextlib.ExitStack() as stack:
            tools = Tree(args.tools, stack)
            before = scan(tools)
            verify(Tree(args.bundle, stack), tools)
            require(scan(tools) == before)
    print("离线包完整性验证通过；身份失效与异机恢复仍须独立执行。")

if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, TypeError, KeyError, RecursionError, subprocess.SubprocessError):
        print("离线包操作失败；未覆盖既有目标，已产生的不完整目录予以保留。", file=sys.stderr)
        sys.exit(1)
