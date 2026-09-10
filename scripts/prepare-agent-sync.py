#!/usr/bin/env python3
"""生成独立、可检查的 macOS 自动同步运行目录；不加载 launchd、不连接服务器。"""
import argparse
import os
from pathlib import Path
import plistlib
import shutil
import sys


LABEL = "me.studiolo.agent-sync"
DEFAULT_VAULT = "/Users/maxsong/Documents/Max's Secret Place"
DEFAULT_HOST = "maxsong@192.168.100.169"
DEFAULT_REMOTE = "/home/maxsong/.local/share/studiolo-agent-replica"


def prepare(output, vault, host, remote, python):
    """将本次代码固定到输出目录，避免之后切分支改变后台任务行为。"""
    output, vault, python = Path(output), Path(vault), Path(python)
    if not output.is_absolute() or output.exists() or output.is_symlink():
        raise ValueError("输出目录必须是尚不存在的绝对路径")
    if not vault.is_absolute() or not vault.is_dir() or vault.is_symlink():
        raise ValueError("Vault 必须是既有实体目录的绝对路径")
    if not python.is_absolute() or not python.is_file() or not os.access(python, os.X_OK):
        raise ValueError("Python 必须是可执行文件的绝对路径")
    if not host or host.startswith("-") or any(c.isspace() for c in host):
        raise ValueError("SSH 主机必须是单个非选项参数")
    if not remote.startswith("/") or ".." in Path(remote).parts:
        raise ValueError("远端副本根必须是无父目录跳转的绝对路径")
    source = Path(__file__).resolve().parent
    scripts = [source / "sync-agent-vault.sh", *sorted(source.glob("replica_*.py"))]
    if len(scripts) < 2 or any(not p.is_file() or p.is_symlink() for p in scripts):
        raise ValueError("同步源文件缺失，无法生成独立运行目录")
    # mkdir 不覆盖旧输出；之后发生错误时只删除本次创建的输出。
    output.mkdir(mode=0o700)
    try:
        for name in ("scripts", "state", "logs"):
            (output / name).mkdir(mode=0o700)
        for script in scripts:
            dest = output / "scripts" / script.name
            shutil.copyfile(script, dest)
            dest.chmod(0o600)
        for name in ("stdout.log", "stderr.log"):
            path = output / "logs" / name
            path.touch(mode=0o600)
        plist = {
            "Label": LABEL,
            "ProgramArguments": ["/bin/bash", str(output / "scripts/sync-agent-vault.sh")],
            "EnvironmentVariables": {
                "STUDIOLO_VAULT": str(vault), "STUDIOLO_HOST": host,
                "STUDIOLO_REMOTE_REPLICA": remote,
                "STUDIOLO_PYTHON": str(python),
                "STUDIOLO_SYNC_STATE_DIR": str(output / "state"),
                "PATH": "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin",
            },
            "RunAtLoad": True,
            "StartInterval": 60,
            "ThrottleInterval": 60,
            "ProcessType": "Background",
            "Umask": 0o077,
            "StandardOutPath": str(output / "logs/stdout.log"),
            "StandardErrorPath": str(output / "logs/stderr.log"),
        }
        (output / f"{LABEL}.plist").write_bytes(plistlib.dumps(plist, sort_keys=False))
        (output / f"{LABEL}.plist").chmod(0o600)
    except BaseException:
        shutil.rmtree(output)
        raise
    return output / f"{LABEL}.plist"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, help="尚不存在的绝对输出目录")
    parser.add_argument("--vault", default=DEFAULT_VAULT)
    parser.add_argument("--host", default=DEFAULT_HOST)
    parser.add_argument("--remote-root", default=DEFAULT_REMOTE)
    parser.add_argument("--python", default=sys.executable)
    args = parser.parse_args()
    try:
        plist = prepare(args.output, args.vault, args.host, args.remote_root, args.python)
    except (OSError, ValueError):
        print("生成失败：请检查绝对路径、输出不存在及完整同步源码。", file=sys.stderr)
        return 1
    print(f"已生成：{plist}\n尚未安装或启用自动同步；启用步骤见 docs/m3/replica-operations.md。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
