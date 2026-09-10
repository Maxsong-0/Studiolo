#!/usr/bin/env python3
"""单次 Mac 捕获/SSH 推送；只输出批次摘要，绝不输出资料正文。"""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import shlex
import signal
import subprocess
import sys
import tempfile
import time

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


def main(argv=None):
    parser = argparse.ArgumentParser(description="稳定捕获并同步 Studiolo 白名单资料")
    parser.add_argument("--dry-run", action="store_true", help="只在本地检查，不调用 SSH")
    args = parser.parse_args(argv)
    source = os.environ.get("STUDIOLO_VAULT", "/Users/maxsong/Documents/Max's Secret Place")
    host = os.environ.get("STUDIOLO_HOST", "maxsong@192.168.100.169")
    remote = os.environ.get("STUDIOLO_REMOTE_REPLICA", "/home/maxsong/.local/share/studiolo-agent-replica")
    temp_base = "/private/tmp" if sys.platform == "darwin" else tempfile.gettempdir()
    state_path = os.environ.get("STUDIOLO_SYNC_STATE_DIR", temp_base + "/studiolo-agent-sync-" + str(os.getuid()))
    os.umask(0o077)
    state_fd = lock_fd = None
    signal.signal(signal.SIGALRM, expired)
    signal.alarm(120)
    started = time.monotonic()
    status = {"status": "failed", "last_error": "validation_failed"}
    try:
        require(bool(re.fullmatch(r"[A-Za-z0-9_][A-Za-z0-9_.@:\[\]-]*", host)))
        require(os.path.isabs(remote) and "\x00" not in remote)
        state_fd = open_root(state_path, create=True, private=True)
        key = digest(encode([source, host, remote]))[:32]
        lock_fd = os.open("sync-" + key + ".lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW,
                          0o600, dir_fd=state_fd)
        check_node(os.fstat(lock_fd), False, private=True)
        require(os.fstat(lock_fd).st_dev == os.fstat(state_fd).st_dev)
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        captured = capture(source)
        raw, bodies, expected, observed = captured
        binding = os.stat("sync-" + key + ".lock", dir_fd=state_fd, follow_symlinks=False)
        check_node(binding, False, private=True)
        require((binding.st_dev, binding.st_ino) == (os.fstat(lock_fd).st_dev, os.fstat(lock_fd).st_ino))
        summary = {"source_digest": digest(raw), "source_observed_at_ms": observed,
                   "files": len(bodies), "bytes": sum(len(b) for b in bodies.values()),
                   "directories": sum(v[0] == "dir" for v in expected.values())}
        if args.dry_run:
            status = {"status": "dry_run", **summary}
        else:
            status["last_error"] = "transfer_failed"
            with archive(captured) as stream, tempfile.TemporaryFile() as output:
                result = subprocess.run(
                    ["ssh", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
                     "-o", "ConnectTimeout=15", "--", host, remote_command(remote)],
                    stdin=stream, stdout=output, stderr=subprocess.DEVNULL,
                    timeout=max(1, 120 - (time.monotonic() - started)), check=False)
                require(result.returncode == 0)
                output.seek(0)
                response = load_json(output.read(8193))
                accept_response(response, summary["source_digest"], observed)
                status = {**response, **summary}
        print(json.dumps(status, ensure_ascii=False, separators=(",", ":")))
        return 0
    except (OSError, ValueError, subprocess.SubprocessError, TimeoutError, RecursionError):
        print(json.dumps(status, separators=(",", ":")), file=sys.stderr)
        return 1
    finally:
        signal.alarm(0)
        if lock_fd is not None:
            os.close(lock_fd)
        if state_fd is not None:
            os.close(state_fd)


if __name__ == "__main__":
    sys.exit(main())
