#!/usr/bin/env python3
"""Linux 冻结人侧快照：调用方必须先停止全部写者；backup 不保证跨库一致性。

只封装 Auth/Audit、配置、完整 release 和外部 age 密文；不解密、不执行包内代码。
失败保留 INCOMPLETE。校验不是签名、因子可用性或跨库一致性的证明。
部署时必须从独立可信来源整组复制本入口及三个 human_snapshot_* 配套模块。
"""
import argparse
import os
import sqlite3
import stat
import sys
import types

HELPERS = ("human_snapshot_io", "human_snapshot_format", "human_snapshot_core")


def load_trusted_helpers():
    """固定入口自身目录；不经当前工作目录或 PYTHONPATH 寻找配套模块。"""
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    path = os.path.abspath(__file__)
    parent, entry = os.path.split(path)
    fd = os.open("/", flags)
    try:
        allowed = {0, os.geteuid()} | ({1000} if os.geteuid() == 0 else set())
        parts = parent.split("/")[1:]
        for index, part in enumerate(parts):
            child = os.open(part, flags, dir_fd=fd)
            os.close(fd)
            fd = child
            st = os.fstat(fd)
            mode = stat.S_IMODE(st.st_mode)
            if st.st_uid not in allowed or mode & 0o6000 or (
                mode & 0o022 and not (index != len(parts) - 1 and st.st_uid == 0 and mode & stat.S_ISVTX)
            ):
                raise ValueError("untrusted_tool_directory")
        owner = os.fstat(fd).st_uid
        for name in (entry, *(module + ".py" for module in HELPERS)):
            item = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=fd)
            try:
                before = os.fstat(item)
                if (not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_uid != owner
                        or before.st_dev != os.fstat(fd).st_dev or before.st_mode & 0o7022 or before.st_size > 128 * 1024):
                    raise ValueError("untrusted_tool_file")
                with os.fdopen(os.dup(item), "rb") as stream:
                    raw = stream.read(128 * 1024 + 1)
                def identity(value):
                    return (value.st_dev, value.st_ino, value.st_mode, value.st_uid, value.st_gid,
                            value.st_nlink, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
                if (len(raw) != before.st_size or identity(before) != identity(os.fstat(item))
                        or identity(before) != identity(os.stat(name, dir_fd=fd, follow_symlinks=False))):
                    raise ValueError("changed_tool_file")
                if name != entry:
                    module = types.ModuleType(name[:-3])
                    module.__file__ = parent + "/" + name
                    sys.modules[module.__name__] = module
                    exec(compile(raw, module.__file__, "exec"), module.__dict__)
            finally:
                os.close(item)
    finally:
        os.close(fd)


try:
    load_trusted_helpers()
except (OSError, ValueError, TypeError, SyntaxError):
    print("human_snapshot_failed：可信工具组不可用。", file=sys.stderr)
    raise SystemExit(1) from None

from human_snapshot_core import create, verify


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("create", help="调用方已共同停写；不会自行停止服务")
    for name in ("auth-dir", "audit-dir", "config", "release", "portable-factor", "snapshot-id", "output"):
        command.add_argument("--" + name, required=True)
    commands.add_parser("verify").add_argument("bundle")
    args = parser.parse_args()
    os.umask(0o077)
    create(args) if args.command == "create" else verify(args.bundle)
    print("human_snapshot_complete；仅文件与快照结构校验，不证明因子可用或跨库一致。")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, TypeError, KeyError, RecursionError, sqlite3.Error):
        print("human_snapshot_failed；未覆盖既有目标，新建失败目录保留INCOMPLETE。", file=sys.stderr)
        sys.exit(1)
