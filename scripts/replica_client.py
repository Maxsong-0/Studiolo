#!/usr/bin/env python3
"""同步捕获/回执校验；SSH 由入口 bash 直接发起，避免 Python 的后台网络身份。"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import sys
import tempfile

from replica_capture import archive, capture
from replica_common import check_node, digest, encode, integer, load_json, open_root, require

MODULES = ("replica_common", "replica_archive", "replica_cleanup", "replica_store", "replica_receiver")


def remote_command(root):
    directory = Path(__file__).absolute().parent
    code = "import sys,types\n"
    for name in MODULES:
        source = (directory / (name + ".py")).read_text(encoding="utf-8")
        code += "m=types.ModuleType(%r);sys.modules[%r]=m;exec(%r,m.__dict__)\n" % (name, name, source)
    code += "sys.exit(m.main())\n"
    return shlex.join(["python3", "-B", "-c", code, "--root", root])


def expired(_signal, _frame):
    raise TimeoutError("transfer_failed")


def accept_response(response, sha, observed):
    """只认可本次捕获的封闭回执，拒绝反射额外字段或复用旧确认时间。"""
    require(isinstance(response, dict) and set(response) == {"status", "receipt", "reused"})
    require(response["status"] == "ready" and type(response["reused"]) is bool)
    receipt = response["receipt"]
    require(isinstance(receipt, dict) and set(receipt) == {
        "generation", "source_digest", "source_observed_at_ms", "received_at_ms"})
    require(isinstance(receipt["generation"], str) and re.fullmatch("[a-f0-9]{32}", receipt["generation"]))
    require(receipt["source_digest"] == sha and integer(receipt["source_observed_at_ms"]))
    require(receipt["source_observed_at_ms"] == observed)
    require(integer(receipt["received_at_ms"]) and receipt["received_at_ms"] >= observed)
    return response


def private_file(directory, name, mode):
    flags = os.O_NOFOLLOW | os.O_NONBLOCK
    flags |= os.O_WRONLY | os.O_CREAT | os.O_EXCL if mode == "wb" else os.O_RDONLY
    fd = os.open(name, flags, 0o600, dir_fd=directory)
    try:
        check_node(os.fstat(fd), False, private=True)
        return os.fdopen(fd, mode)
    except BaseException:
        os.close(fd)
        raise


def main(argv=None):
    parser = argparse.ArgumentParser(description="稳定捕获并同步 Studiolo 白名单资料（入口 sync-agent-vault.sh）")
    modes = parser.add_mutually_exclusive_group(required=True)
    modes.add_argument("--dry-run", action="store_true", help="只在本地检查，不调用 SSH")
    modes.add_argument("--lock-path", action="store_true", help=argparse.SUPPRESS)
    modes.add_argument("--prepare", metavar="DIRECTORY", help=argparse.SUPPRESS)
    modes.add_argument("--finish", metavar="DIRECTORY", help=argparse.SUPPRESS)
    parser.add_argument("--ssh-status", type=int, help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    source = os.environ.get("STUDIOLO_VAULT", "/Users/maxsong/Documents/Max's Secret Place")
    host = os.environ.get("STUDIOLO_HOST", "maxsong@192.168.100.169")
    remote = os.environ.get("STUDIOLO_REMOTE_REPLICA", "/home/maxsong/.local/share/studiolo-agent-replica")
    temp_base = "/private/tmp" if sys.platform == "darwin" else tempfile.gettempdir()
    state_path = os.environ.get("STUDIOLO_SYNC_STATE_DIR", temp_base + "/studiolo-agent-sync-" + str(os.getuid()))
    os.umask(0o077)
    state_fd = lock_fd = work_fd = None
    signal.signal(signal.SIGALRM, expired)
    signal.alarm(120)
    status = {"status": "failed", "last_error": "validation_failed"}
    try:
        require(bool(re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.@:\[\]-]*", host)))
        require(os.path.isabs(remote) and "\x00" not in remote)
        require("\n" not in state_path and "\r" not in state_path)
        state_fd = open_root(state_path, create=True, private=True)
        key = digest(encode([source, host, remote]))[:32]
        name = "sync-" + key + ".lock"
        lock_fd = os.open(name, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK,
                          0o600, dir_fd=state_fd)
        check_node(os.fstat(lock_fd), False, private=True)
        require(os.fstat(lock_fd).st_dev == os.fstat(state_fd).st_dev)
        if args.lock_path:
            print(state_path + "/" + name)
            return 0
        if args.prepare or args.finish:
            # fd 9 由 bash 只读打开；先证明它仍是安全根中的那把锁，再 flock。
            # flock 属于共享 open-file-description：Python 退出后 bash 仍持锁。
            inherited = os.fstat(9)
            check_node(inherited, False, private=True)
            require((inherited.st_dev, inherited.st_ino) ==
                    (os.fstat(lock_fd).st_dev, os.fstat(lock_fd).st_ino))
            os.close(lock_fd)
            lock_fd = os.dup(9)
            work_fd = open_root(args.prepare or args.finish, private=True)
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if args.finish:
            status["last_error"] = "transfer_failed"
            require(args.ssh_status is not None)
            status["ssh_exit_code"] = args.ssh_status
            require(args.ssh_status == 0)
            with private_file(work_fd, "summary.json", "rb") as saved:
                summary = load_json(saved.read(8193))
            with private_file(work_fd, "response.json", "rb") as output:
                raw = output.read(8193)
            require(len(raw) <= 8192)
            response = accept_response(load_json(raw), summary["source_digest"],
                                       summary["source_observed_at_ms"])
            status = {**response, **summary}
        else:
            captured = capture(source)
            raw, bodies, expected, observed = captured
            binding = os.stat(name, dir_fd=state_fd, follow_symlinks=False)
            check_node(binding, False, private=True)
            require((binding.st_dev, binding.st_ino) == (os.fstat(lock_fd).st_dev, os.fstat(lock_fd).st_ino))
            summary = {"source_digest": digest(raw), "source_observed_at_ms": observed,
                       "files": len(bodies), "bytes": sum(len(b) for b in bodies.values()),
                       "directories": sum(v[0] == "dir" for v in expected.values())}
            if args.dry_run:
                status = {"status": "dry_run", **summary}
            else:
                with archive(captured) as stream, private_file(work_fd, "archive.tar", "wb") as output:
                    shutil.copyfileobj(stream, output)
                for filename, raw in (("host", host.encode()), ("command", remote_command(remote).encode()),
                                      ("summary.json", encode(summary))):
                    with private_file(work_fd, filename, "wb") as output:
                        output.write(raw)
                return 0
        print(json.dumps(status, ensure_ascii=False, separators=(",", ":")))
        return 0
    except (OSError, ValueError, TimeoutError, RecursionError):
        print(json.dumps(status, separators=(",", ":")), file=sys.stderr)
        return 1
    finally:
        signal.alarm(0)
        for fd in (lock_fd, state_fd, work_fd):
            if fd is not None:
                os.close(fd)


if __name__ == "__main__":
    sys.exit(main())
