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
import subprocess
import sys
import tempfile
import time
import uuid

from replica_archive import transfer_deadline
import replica_capacity as capacity
from replica_capture import archive, capture, missing_registered, transfer_mode
from replica_common import (MAX_MANIFEST, Invalid, check_node, digest, encode, integer, load_json,
                            manifest, open_root, read_file, require)

MODULES = ("replica_common", "replica_archive", "replica_cleanup", "replica_store", "replica_receiver")
# 同步默认值只在这里定义一份，prepare-agent-sync.py 也从这里取。
# vault 按家目录拼：两台机器的 vault 都在家目录下同一相对位置（Mac /Users/maxsong、DGX /home/maxsong），
# 写死 /Users/... 在 DGX 上指向不存在的路径。副本仍只由 Mac 推（2026-10-04 用户拍板），这里只去掉写死。
DEFAULT_VAULT = str(Path.home() / "Documents" / "Max's Secret Place")
DEFAULT_HOST = "maxsong@192.168.100.169"
DEFAULT_REMOTE = "/home/maxsong/.local/share/studiolo-agent-replica"  # 服务器 169 上的路径，不随本机变
# 容量提醒的去重状态（state/capacity-<key>.json）最大字节数；四条各一小段，远用不完。
ALERT_STATE_LIMIT = 4096
# 发一条系统通知最多等多久；通知发不出去不拦同步，下一轮再试。
NOTIFY_TIMEOUT_SECS = 10
# osascript 的脚本固定这三行，标题和正文只作为参数交给 run 处理器：文字里有引号或 AppleScript 也不会被执行。
NOTIFY_SCRIPT = ("on run argv", "display notification (item 2 of argv) with title (item 1 of argv)", "end run")


def remote_command(root, deadline):
    directory = Path(__file__).absolute().parent
    code = "import sys,types\n"
    for name in MODULES:
        source = (directory / (name + ".py")).read_text(encoding="utf-8")
        code += "m=types.ModuleType(%r);sys.modules[%r]=m;exec(%r,m.__dict__)\n" % (name, name, source)
    code += "sys.exit(m.main())\n"
    return shlex.join(["python3", "-B", "-c", code, "--root", root, "--deadline", str(deadline)])


def load_base(state_fd, name):
    """读上一轮已被服务器确认的清单，返回 (摘要, expected)；没有或已不合现行白名单则为 None。

    文件本身不安全（链接、权限、大小）照常报错；只有「清单按现行规则不再成立」才静默作废，
    例如白名单变了——此时改发整份，服务器也会拒绝用旧基线的请求。
    """
    try:
        raw, _ = read_file(state_fd, name, MAX_MANIFEST)
    except FileNotFoundError:
        return None
    try:
        _, expected = manifest(raw)
    except Invalid:
        drop_base(state_fd, name)
        return None
    return digest(raw), expected


def save_private(state_fd, name, raw):
    """在私有状态目录里原子替换一个 0600 文件（基线、容量提醒状态共用）。"""
    temp = ".state-" + uuid.uuid4().hex
    try:
        with private_file(state_fd, temp, "wb") as output:
            output.write(raw)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temp, name, src_dir_fd=state_fd, dst_dir_fd=state_fd)
        os.fsync(state_fd)
    finally:
        try:
            os.unlink(temp, dir_fd=state_fd)
        except FileNotFoundError:
            pass


def drop_base(state_fd, name):
    try:
        os.unlink(name, dir_fd=state_fd)
        os.fsync(state_fd)
    except FileNotFoundError:
        pass


def notify(title, body):
    """给 Mac 用户发一条系统通知，返回 sent／failed／unavailable；不抛异常，通知失败不拦同步。

    osascript 按 PATH 找（launchd 任务的 PATH 以 /usr/bin 开头，同 SSH 的找法）；不是 macOS 就是 unavailable。
    """
    program = shutil.which("osascript")
    if program is None:
        return "unavailable"
    command = [program, *(part for line in NOTIFY_SCRIPT for part in ("-e", line)), title, body]
    try:
        done = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, timeout=NOTIFY_TIMEOUT_SECS, check=False)
    except (OSError, subprocess.SubprocessError):
        return "failed"
    return "sent" if done.returncode == 0 else "failed"


def load_alert_state(state_fd, name):
    """上一轮记下的容量提醒状态。文件本身不安全（链接、权限、大小）照常报错；内容读不懂当作从没提醒过。"""
    try:
        raw, _ = read_file(state_fd, name, ALERT_STATE_LIMIT)
    except FileNotFoundError:
        return {}
    try:
        return capacity.parse_state(load_json(raw))
    except (ValueError, RecursionError):  # 嵌套很深的 JSON 抛的是 RecursionError
        return {}


def alert(state_fd, name, observed, notice, now_ms, dry_run, send=notify):
    """按去重状态决定这一轮发不发容量提醒，返回写进结果的 capacity_alert；一条都没到线时为 None。

    observed、notice 见 replica_capacity.plan 与 warn_notice／exceeded_notice。预演只说会不会发
    （would_send／suppressed），不发、不写状态。真发时：sent 才记下这次提醒；failed／unavailable 不记，
    下一轮重试，但退到线下的那几条照样忘掉。
    """
    state = load_alert_state(state_fd, name)
    due, updated = capacity.plan(state, observed, now_ms)
    if dry_run:
        return None if notice is None else "would_send" if due else "suppressed"
    outcome = None if notice is None else "suppressed"
    if due:
        outcome = send(*notice)
        if outcome != "sent":
            updated = capacity.forget(state, observed)
    if updated != state:
        save_private(state_fd, name, encode(capacity.dump_state(updated)))
    return outcome


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


def require_lock_binding(state_fd, name, lock_fd):
    """捕获完再核一次：状态目录里那把锁仍是本进程持有的同一个文件，之后才写状态目录。"""
    binding = os.stat(name, dir_fd=state_fd, follow_symlinks=False)
    check_node(binding, False, private=True)
    require((binding.st_dev, binding.st_ino) == (os.fstat(lock_fd).st_dev, os.fstat(lock_fd).st_ino))


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
    source = os.environ.get("STUDIOLO_VAULT", DEFAULT_VAULT)
    host = os.environ.get("STUDIOLO_HOST", DEFAULT_HOST)
    remote = os.environ.get("STUDIOLO_REMOTE_REPLICA", DEFAULT_REMOTE)
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
        base_name = "base-" + key + ".json"
        alert_name = "capacity-" + key + ".json"
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
            value = load_json(raw)
            if value == {"status": "base_mismatch"}:
                # 服务器现代次不是本端基线（被重置、上一轮回执丢失或现代次损坏）：清基线，下一轮整份自愈。
                drop_base(state_fd, base_name)
                status = {"status": "failed", "last_error": "base_mismatch"}
                print(json.dumps(status, separators=(",", ":")), file=sys.stderr)
                return 1
            response = accept_response(value, summary["source_digest"], summary["source_observed_at_ms"])
            with private_file(work_fd, "manifest.json", "rb") as saved:
                manifest_raw = saved.read(MAX_MANIFEST + 1)
            require(len(manifest_raw) <= MAX_MANIFEST and digest(manifest_raw) == summary["source_digest"])
            save_private(state_fd, base_name, manifest_raw)
            status = {**response, **summary}
        else:
            missing = missing_registered(source)
            if missing:
                # 整批照旧不发，只把原因和个数说清楚；逐个列出缺哪些、怎么改：
                # python3 scripts/project_registry.py check --vault <库根>（docs/m3/replica-operations.md）。
                status = {"status": "failed", "last_error": "registered_directory_missing",
                          "missing": len(missing)}
                print(json.dumps(status, separators=(",", ":")), file=sys.stderr)
                return 1
            try:
                captured = capture(source)
            except capacity.LimitExceeded as over:
                # 越线（BUG-039）：整批照旧不发，日志点名是哪一条，并给 Mac 用户发系统通知（T-217）。
                require_lock_binding(state_fd, name, lock_fd)
                notice = capacity.exceeded_notice(over.gauge)
                # 先把原因定下来再碰去重状态：状态文件不安全或写不了时，结果照样点名哪一条越线，
                # 不被外层兜成笼统的 validation_failed（那正是 BUG-039 看不出原因的样子）。
                status = {"status": "failed", "last_error": "capacity_exceeded", "exceeded": over.gauge,
                          "capacity_notice": notice[1]}
                try:
                    status["capacity_alert"] = alert(state_fd, alert_name, {over.gauge: "exceeded"}, notice,
                                                     time.time_ns() // 1_000_000, args.dry_run)
                except (OSError, ValueError):
                    status["capacity_alert"] = "state_error"
                print(json.dumps(status, ensure_ascii=False, separators=(",", ":")), file=sys.stderr)
                return 1
            raw, bodies, expected, observed, usage = captured
            require_lock_binding(state_fd, name, lock_fd)
            summary = {"source_digest": digest(raw), "source_observed_at_ms": observed,
                       "files": len(bodies), "bytes": sum(len(b) for b in bodies.values()),
                       "directories": sum(v[0] == "dir" for v in expected.values()),
                       "capacity": capacity.report(usage)}
            # 四条上限到 80% 起：结果里写一句提醒，并按去重给 Mac 用户发系统通知（T-217，不经 agent 的读授权）。
            notice = capacity.warn_notice(summary["capacity"])
            outcome = alert(state_fd, alert_name, capacity.levels(summary["capacity"]), notice, observed,
                            args.dry_run)
            if notice is not None:
                summary.update(capacity_notice=notice[1], capacity_alert=outcome)
            if args.dry_run:
                status = {"status": "dry_run", **summary}
            else:
                base = load_base(state_fd, base_name)
                with archive(captured, base) as stream, private_file(work_fd, "archive.tar", "wb") as output:
                    shutil.copyfileobj(stream, output)
                    sent = output.tell()
                deadline = transfer_deadline(sent)
                summary.update(mode=transfer_mode(captured, base), sent_bytes=sent)
                for filename, raw_value in (("host", host.encode()), ("manifest.json", raw),
                                            ("command", remote_command(remote, deadline).encode()),
                                            ("deadline", str(deadline).encode()),
                                            ("summary.json", encode(summary))):
                    with private_file(work_fd, filename, "wb") as output:
                        output.write(raw_value)
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
