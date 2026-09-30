#!/usr/bin/python3 -I
"""仅供用户私有前台终端运行的生产配对入口；不执行同步。"""

import json
import os
from pathlib import Path
import pwd
import re
import stat
import subprocess
import sys

USER, UID, GID = "studiolo-sync", 999, 983
MOUNT = Path("/var/lib/studiolo-sync")
MIRROR = MOUNT / "bridge/mirror"
CONFIG = MOUNT / "config"
IMAGE = "/var/lib/studiolo-sync.img"
RECEIPT = Path("/var/lib/studiolo-bridge-p3/accepted.json")
NODE = Path("/opt/studiolo/node/bin/node")
CLI = Path("/opt/studiolo/obsidian-headless/lib/node_modules/obsidian-headless/cli.js")
VAULT, DEVICE = "Max's Secret Place", "studiolo-169-bridge"
EXCLUDED = ("School", "日记", "大学申请", "周报+日报", "Ninth English")
FILE_TYPES = ("image", "audio", "video", "pdf", "unsupported")
ENV = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8", "HOME": str(MOUNT),
       "XDG_CONFIG_HOME": str(CONFIG)}
LIMIT = 64 * 1024


class Refused(Exception):
    """只携带固定阶段和退出码，不保留子进程正文。"""

    def __init__(self, stage, code=1):
        self.stage, self.code = stage, code


def require(condition, stage):
    if not condition:
        raise Refused(stage)


def terminal():
    require(sys.platform == "linux" and os.getuid() == os.geteuid() == 0, "operator")
    require(all(stream.isatty() for stream in (sys.stdin, sys.stdout, sys.stderr)), "tty")
    fd = os.open("/dev/tty", os.O_RDWR | os.O_NOCTTY | os.O_CLOEXEC)
    if not os.isatty(fd):
        os.close(fd)
        raise Refused("tty")
    return fd


def chain(path):
    """逐级 lstat，连叶节点也不接受符号链接。"""
    require(path.is_absolute(), "path")
    entries = []
    for node in (*reversed(path.parents), path):
        info = node.lstat()
        require(not stat.S_ISLNK(info.st_mode), "path")
        if node != path:
            require(stat.S_ISDIR(info.st_mode), "path")
        entries.append(info)
    return entries


def trusted_file(path):
    entries = chain(path)
    require(all(info.st_uid == 0 and not info.st_mode & 0o022 for info in entries), "runtime")
    require(stat.S_ISREG(entries[-1].st_mode), "runtime")
    return entries[-1]


def private_directory(path):
    info = chain(path)[-1]
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == UID and info.st_gid == GID
            and stat.S_IMODE(info.st_mode) == 0o700, "directory")
    return info


def object_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "json")
        result[key] = value
    return result


def document(raw):
    require(len(raw) <= LIMIT, "json")
    try:
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=object_pairs)
    except (ValueError, UnicodeError, RecursionError):
        raise Refused("json") from None
    require(type(value) is dict, "json")
    return value


def accepted():
    """只检查既有 P3 回执的状态；不创建、修复或回显回执。"""
    before = trusted_file(RECEIPT)
    require(stat.S_IMODE(before.st_mode) == 0o600 and before.st_nlink == 1
            and before.st_size <= LIMIT, "p3")
    fd = os.open(RECEIPT, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    with os.fdopen(fd, "rb") as stream:
        opened = os.fstat(stream.fileno())
        # 读取本身可能推进 atime；稳定性只比较身份、权限、长度与内容变更时间。
        def stamp(info):
            return (info.st_dev, info.st_ino, info.st_mode, info.st_uid, info.st_gid,
                    info.st_nlink, info.st_size, info.st_mtime_ns, info.st_ctime_ns)
        require(stamp(opened) == stamp(before), "p3")
        raw = stream.read(LIMIT + 1)
        require(stamp(os.fstat(stream.fileno())) == stamp(opened)
                and stamp(RECEIPT.lstat()) == stamp(before), "p3")
    require(document(raw).get("status") == "accepted", "p3")


def loop_device(mountinfo, device):
    """以内核挂载表与目录设备号绑定固定 loop 根，拒绝子目录 bind。"""
    matches = []
    for line in mountinfo.splitlines():
        left, separator, right = line.partition(" - ")
        fields, source = left.split(), right.split()
        if separator and len(fields) >= 6 and len(source) >= 2 and fields[4] == str(MOUNT):
            require(fields[3] == "/" and fields[2] == device, "mount")
            require(re.fullmatch(r"/dev/loop[0-9]+", source[1]) is not None, "mount")
            matches.append(source[1])
    require(len(matches) == 1, "mount")


def preflight():
    account = pwd.getpwnam(USER)
    require((account.pw_uid, account.pw_gid) == (UID, GID), "identity")
    require(set(os.getgrouplist(USER, GID)) == {GID}, "identity")
    node, cli = trusted_file(NODE), trusted_file(CLI)
    require(node.st_mode & 0o001 and cli.st_mode & 0o004, "runtime")
    accepted()
    mount = chain(MOUNT)[-1]
    mirror, config = private_directory(MIRROR), private_directory(CONFIG)
    require(stat.S_ISDIR(mount.st_mode) and mount.st_dev == mirror.st_dev == config.st_dev, "mount")
    device = f"{os.major(mount.st_dev)}:{os.minor(mount.st_dev)}"
    loop_device(Path("/proc/self/mountinfo").read_text(), device)
    backing = Path(f"/sys/dev/block/{device}/loop/backing_file").read_text().strip()
    require(backing == IMAGE, "mount")
    with os.scandir(MIRROR) as entries:
        require(next(entries, None) is None, "mirror-not-empty")


def command(stage):
    """只有三个固定子命令，口令只能由用户在继承的 TTY 内输入。"""
    steps = {
        "login": ["login"],
        "setup": ["sync-setup", "--vault", VAULT, "--path", str(MIRROR),
                  "--device-name", DEVICE],
        "configure": ["sync-config", "--path", str(MIRROR), "--mode", "mirror-remote",
                      "--conflict-strategy", "conflict", "--excluded-folders", ",".join(EXCLUDED),
                      "--file-types", ",".join(FILE_TYPES), "--configs", "",
                      "--device-name", DEVICE, "--json"],
    }
    require(stage in steps, "command")
    return ["/usr/sbin/runuser", "-u", USER, "--", "/usr/bin/env", "-i",
            *(f"{key}={value}" for key, value in ENV.items()), str(NODE), str(CLI), *steps[stage]]


def run(stage):
    options = {"env": dict(ENV), "cwd": "/", "check": False}
    if stage == "configure":
        # 配置结果只留内存；stderr 丢弃，避免异常日志成为凭据出口。
        options.update(stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                       stderr=subprocess.DEVNULL, timeout=60)
    try:
        result = subprocess.run(command(stage), **options)
    except subprocess.TimeoutExpired:
        raise Refused(stage, 124) from None
    except OSError:
        raise Refused(stage, 127) from None
    require(type(result.returncode) is int, stage)
    if result.returncode != 0:
        raise Refused(stage, result.returncode)
    return result.stdout if stage == "configure" else None


def verify(raw):
    value = document(raw)
    expected = {"vaultName": VAULT, "vaultPath": str(MIRROR), "syncMode": "mirror-remote",
                "conflictStrategy": "conflict", "deviceName": DEVICE, "configDir": ".obsidian",
                "configs": []}
    require(set(value) == set(expected) | {"vaultId", "fileTypes", "excludedFolders"}, "config")
    require(all(type(value[key]) is type(wanted) and value[key] == wanted
                for key, wanted in expected.items()), "config")
    identity = value["vaultId"]
    require(type(identity) is str and re.fullmatch(r"[0-9a-f]{32}", identity) is not None
            and identity != "0" * 32, "config")
    for key, approved in (("fileTypes", FILE_TYPES), ("excludedFolders", EXCLUDED)):
        actual = value[key]
        require(type(actual) is list and len(actual) == len(approved)
                and all(type(item) is str for item in actual), "config")
        require(set(actual) == set(approved), "config")


def main(args=None):
    fd, stage = None, "arguments"
    try:
        require(not (sys.argv[1:] if args is None else args), stage)
        os.environ.clear()
        os.environ.update(ENV)
        os.umask(0o077)
        stage = "tty"
        fd = terminal()
        stage = "preflight"
        preflight()
        for stage in ("login", "setup", "configure"):
            output = run(stage)
        verify(output)
        print("pairing_configured=true mode=mirror-remote excluded_folders=5 file_types=5 configs=0")
        return 0
    except Refused as error:
        print(f"pairing_failed stage={error.stage} exit_code={error.code}", file=sys.stderr)
        return 1
    except (OSError, KeyError, ValueError, KeyboardInterrupt):
        print(f"pairing_failed stage={stage} exit_code=1", file=sys.stderr)
        return 1
    finally:
        if fd is not None:
            os.close(fd)


if __name__ == "__main__":
    sys.exit(main())
