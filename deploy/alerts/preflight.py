#!/usr/bin/env python3
"""候选单元的只读启动预检：只查路径、权限和来源，不读取或打印凭据。"""
import importlib.util
import os
from pathlib import Path
import stat
import sys

PROTECTION = importlib.util.spec_from_file_location("residency", Path(__file__).with_name("residency.py"))
RESIDENCY = importlib.util.module_from_spec(PROTECTION)
PROTECTION.loader.exec_module(RESIDENCY)

SPEC = importlib.util.spec_from_file_location("private_state", Path(__file__).with_name("private_state.py"))
IO = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(IO)
BASE = Path("/home/maxsong")


def private_file(path):
    directory = IO.private_directory(str(path.parent))
    try:
        descriptor = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK, dir_fd=directory)
        try:
            IO.private_file(descriptor)
            if not 0 < os.fstat(descriptor).st_size <= 16384:
                raise ValueError("configuration_size")
        finally:
            os.close(descriptor)
    finally:
        os.close(directory)


def trusted_release():
    release = Path(__file__).resolve(strict=True).parents[2]
    if release.parent != Path("/opt/studiolo-alert/releases"):
        raise ValueError("release_path")
    for path in (release, *release.parents):
        info = path.stat()
        if info.st_uid != 0 or info.st_mode & 0o022:
            raise ValueError("release_owner")
    required = ["deploy/alerts/run.py", "deploy/alerts/residency.py", "deploy/alerts/probe.py", "deploy/alerts/private_state.py",
                "deploy/alerts/bounded_process.py", "deploy/alerts/heartbeat.py", "deploy/alerts/watch_state.py",
                "scripts/check-agent-replica.py", "scripts/check-agent.py", "bin/studiolo-alert"]
    for name in required:
        path = release / name
        for parent in (path.parent, *path.parent.parents):
            if parent == release: break
            info = parent.lstat()
            if not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o022:
                raise ValueError("release_parent")
        info = path.lstat()
        if not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o022 or info.st_nlink != 1:
            raise ValueError("release_file")
    return release


def main():
    try:
        # systemd 也给 ExecStartPre 注入环境；在接触配置前保护这一进程。
        RESIDENCY.harden()
        if sys.argv[1:] not in (["n108"], ["n169"]):
            raise ValueError("node")
        node = sys.argv[1]
        trusted_release()
        config = BASE / ".config/studiolo-alert"
        private_file(config / (node + ".env"))
        names = ("watch", "audit") if node == "n108" else ("probe", "probe-receipt")
        for name in names:
            os.close(IO.private_directory(str(BASE / ".local/share/studiolo-alert" / name)))
        if node == "n169": private_file(config / "agent-token")
        print("preflight_ok")
        return 0
    except Exception:
        print("alert_preflight_failed", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
