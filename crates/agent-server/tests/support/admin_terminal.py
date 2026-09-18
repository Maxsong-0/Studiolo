"""真实 PTY 驱动实际产品二进制：类型 token 只在真实前台控制终端显示一次。

只使用独立临时私有目录与本次现场签发的虚构 token；不读产品配置、不联网、不留残留。
用法：python3 admin_terminal.py <studiolo-agent 绝对路径> <behaviour|failure|recovery>
"""

import errno
import os
import pty
import re
import select
import shutil
import signal
import subprocess
import sys
import tempfile
import termios
import time
from pathlib import Path

BINARY = str(Path(sys.argv[1]).resolve())
SCENARIO = sys.argv[2]
ENVIRONMENT = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C.UTF-8"}
# 明文形如 `sag1_` + 64 位小写十六进制；另单独扫裸 64 位十六进制，防止前缀被改掉后漏检。
TOKEN = re.compile(rb"sag1_[0-9a-f]{64}")
BARE_HEX = re.compile(rb"[0-9a-f]{64}")
REFUSAL = "拒绝签发类型 token".encode()
NOT_ISSUED = "本次未签发任何 token".encode()
NO_REDIRECT = "不要重定向".encode()
SHOWN_ONCE = "只显示这一次".encode()
CONFIRMED = "已在控制终端显示一次".encode()
TIMEOUT = 25


def fail(message):
    raise AssertionError(message)


def run_pty(arguments, redirect_fd=None, target=None, pipe_fd=None, devnull_stdin=False):
    """在真实 PTY 里运行产品二进制，可把某个标准流改接文件或管道。

    返回 (退出码, 终端上看到的字节, 被改接的那个流收到的字节)。
    """
    ready_read, ready_write = os.pipe()
    capture_read = capture_write = None
    if pipe_fd is not None:
        capture_read, capture_write = os.pipe()
    pid, master = pty.fork()
    if pid == 0:
        try:
            os.close(ready_write)
            if capture_read is not None:
                os.close(capture_read)
            os.read(ready_read, 1)
            os.close(ready_read)
            if devnull_stdin:
                handle = os.open(os.devnull, os.O_RDONLY)
                os.dup2(handle, 0)
                os.close(handle)
            if redirect_fd is not None:
                handle = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
                os.dup2(handle, redirect_fd)
                os.close(handle)
            if pipe_fd is not None:
                os.dup2(capture_write, pipe_fd)
                os.close(capture_write)
            os.execve(BINARY, [BINARY, *arguments], ENVIRONMENT)
        except BaseException:
            os._exit(127)
    os.close(ready_read)
    if capture_write is not None:
        os.close(capture_write)
    original = termios.tcgetattr(master)
    terminal_output = bytearray()
    captured = bytearray()
    status = None
    os.write(ready_write, b"1")
    os.close(ready_write)
    deadline = time.monotonic() + TIMEOUT
    try:
        while time.monotonic() < deadline:
            sources = [master] + ([capture_read] if capture_read is not None else [])
            for source in select.select(sources, [], [], 0.05)[0]:
                try:
                    chunk = os.read(source, 4096)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    continue
                if chunk:
                    (terminal_output if source == master else captured).extend(chunk)
            done, value = os.waitpid(pid, os.WNOHANG)
            if done:
                status = value
                break
        if status is None:
            fail(f"实际 CLI 执行超时：{arguments}")
        # 子进程退出后继续收尾读取，不把「没读到」当作「没泄漏」。
        while select.select([master], [], [], 0.05)[0]:
            try:
                chunk = os.read(master, 4096)
            except OSError as error:
                if error.errno == errno.EIO:
                    break
                raise
            if not chunk:
                break
            terminal_output.extend(chunk)
        if capture_read is not None:
            while select.select([capture_read], [], [], 0.05)[0]:
                chunk = os.read(capture_read, 4096)
                if not chunk:
                    break
                captured.extend(chunk)
        if termios.tcgetattr(master) != original:
            fail("本命令不改终端属性，退出后属性必须与进入前完全一致")
        return os.waitstatus_to_exitcode(status), bytes(terminal_output), bytes(captured)
    finally:
        if status is None:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(master)
        if capture_read is not None:
            os.close(capture_read)


def run_plain(arguments, new_session=False):
    """无 TTY 的普通进程：三个标准流都是管道，可选连控制终端也不给。"""
    result = subprocess.run(
        [BINARY, *arguments],
        capture_output=True,
        env=ENVIRONMENT,
        timeout=TIMEOUT,
        start_new_session=new_session,
        stdin=subprocess.DEVNULL,
    )
    return result.returncode, result.stdout, result.stderr


def prepare():
    # 必须 resolve：私有存储逐层拒绝符号链接，macOS 的临时目录经 /var 软链。
    root = Path(tempfile.mkdtemp(prefix="studiolo-agent-terminal-")).resolve()
    root.chmod(0o700)
    data = root / "data"
    data.mkdir(mode=0o700)
    (data / "audit").mkdir(mode=0o700)
    code, out, err = run_plain(["admin", "init", "--data", str(data)])
    if code != 0:
        fail(f"初始化失败：{out!r}{err!r}")
    return root, data


def types(data):
    code, out, err = run_plain(["admin", "types", "--data", str(data)])
    if code != 0:
        fail(f"admin types 失败：{out!r}{err!r}")
    if TOKEN.search(out) or TOKEN.search(err):
        fail("admin types 泄漏了明文 token")
    return [line.split("\t") for line in out.decode().splitlines() if line.strip()]


def assert_no_token(blob, where, forbidden=()):
    if TOKEN.search(blob):
        fail(f"{where} 出现了明文类型 token")
    if BARE_HEX.search(blob):
        fail(f"{where} 出现了疑似 64 位十六进制秘密")
    for secret in forbidden:
        if secret in blob:
            fail(f"{where} 出现了此前签发的 token")


def issue_on_terminal(data, agent_type):
    code, terminal_output, _ = run_pty(
        ["admin", "type-issue", "--data", str(data), "--type", agent_type]
    )
    if code != 0:
        fail(f"真实控制终端上签发应当成功，实际退出码 {code}：{terminal_output!r}")
    found = TOKEN.findall(terminal_output)
    if len(found) != 1:
        fail(f"控制终端上必须恰好显示一次明文 token，实际 {len(found)} 次")
    for expected in (SHOWN_ONCE, CONFIRMED):
        if expected not in terminal_output:
            fail(f"终端缺少固定说明：{expected.decode()}")
    return found[0], terminal_output


def behaviour(root, data):
    token, _ = issue_on_terminal(data, "claude")
    rows = types(data)
    if len(rows) != 1 or rows[0][0] != "claude" or rows[0][2] != "有效" or not rows[0][1]:
        fail(f"签发后类型状态不对：{rows}")
    # 同一 token 不会被任何非秘密查询再吐出来。
    for command in (["admin", "types"], ["admin", "outages"], ["admin", "reads"]):
        code, out, err = run_plain([*command, "--data", str(data)])
        assert_no_token(out + err, f"{command[1]} 的输出", forbidden=(token,))
    return "真实控制终端上恰好显示一次，非秘密查询不复述"


def failure(root, data):
    target = root / "token.txt"
    attempts = []

    # 1. 完全没有控制终端（CI、systemd、nohup、setsid）。
    code, out, err = run_plain(
        ["admin", "type-issue", "--data", str(data), "--type", "claude"], new_session=True
    )
    attempts.append(("无控制终端", code, out + err, out + err))

    # 2. 有控制终端但三个标准流是管道（被另一个进程包起来跑）。
    code, out, err = run_plain(["admin", "type-issue", "--data", str(data), "--type", "claude"])
    attempts.append(("标准流全是管道", code, out + err, out + err))

    # 3. 终端里 `> token.txt`：正是运维手册里那条命令的形状。
    code, terminal_output, _ = run_pty(
        ["admin", "type-issue", "--data", str(data), "--type", "claude"],
        redirect_fd=1,
        target=str(target),
    )
    written = target.read_bytes()
    attempts.append(("标准输出重定向到文件", code, terminal_output + written, written))
    if TOKEN.search(written) or BARE_HEX.search(written):
        fail("重定向目标文件里出现了 token")

    # 4. 终端里 `| tail -1`：管道同样不许拿到明文。
    code, terminal_output, captured = run_pty(
        ["admin", "type-issue", "--data", str(data), "--type", "claude"], pipe_fd=1
    )
    attempts.append(("标准输出接管道", code, terminal_output + captured, captured))

    # 5. 标准错误被改接文件：三个流必须是同一个终端，少一个就拒绝。
    stderr_target = root / "stderr.txt"
    code, terminal_output, _ = run_pty(
        ["admin", "type-issue", "--data", str(data), "--type", "claude"],
        redirect_fd=2,
        target=str(stderr_target),
    )
    written = stderr_target.read_bytes()
    attempts.append(("标准错误重定向到文件", code, terminal_output + written, written))

    # 6. 标准输入来自 /dev/null：不是交互终端，同样拒绝。
    code, terminal_output, _ = run_pty(
        ["admin", "type-issue", "--data", str(data), "--type", "claude"], devnull_stdin=True
    )
    attempts.append(("标准输入来自 /dev/null", code, terminal_output, terminal_output))

    for name, code, everything, sink in attempts:
        if code == 0:
            fail(f"{name} 必须被拒绝，实际退出码 0")
        assert_no_token(everything, name)
        if REFUSAL not in everything or NOT_ISSUED not in everything:
            fail(f"{name} 缺少明确的拒绝说明：{everything!r}")
        if NO_REDIRECT not in everything:
            fail(f"{name} 缺少可操作的处置建议：{everything!r}")
        assert_no_token(sink, f"{name} 的目标")

    rows = types(data)
    if rows:
        fail(f"任何拒绝路径都不得签发 token，实际留下：{rows}")
    return f"{len(attempts)} 种非控制终端场景各自被拒绝，目标里无 token，且一枚都没签发"


def recovery(root, data):
    first, _ = issue_on_terminal(data, "claude")
    before = types(data)
    if len(before) != 1 or before[0][2] != "有效":
        fail(f"首次签发后应为有效：{before}")

    # 被拒绝的一次调用既不轮换也不吊销既有凭据。
    target = root / "rejected.txt"
    code, terminal_output, _ = run_pty(
        ["admin", "type-issue", "--data", str(data), "--type", "claude"],
        redirect_fd=1,
        target=str(target),
    )
    if code == 0:
        fail("重定向场景必须被拒绝")
    written = target.read_bytes()
    assert_no_token(written, "被拒绝那次的重定向目标", forbidden=(first,))
    assert_no_token(terminal_output, "被拒绝那次的终端输出", forbidden=(first,))
    after = types(data)
    if after != before:
        fail(f"拒绝不得改变库中状态：{before} → {after}")

    # 终端恢复正常后照常轮换，且绝不重复显示上一次那枚 token。
    second, second_output = issue_on_terminal(data, "claude")
    if second == first:
        fail("每次签发都必须是新的随机 token")
    if first in second_output:
        fail("上一次的 token 不得被再次显示")
    rotated = types(data)
    if len(rotated) != 1 or rotated[0][2] != "有效" or rotated[0][1] == before[0][1]:
        fail(f"轮换后签发时间应当前进且仍有效：{before} → {rotated}")
    return "拒绝不留半截状态，终端恢复后轮换成功且不复显旧 token"


SCENARIOS = {"behaviour": behaviour, "failure": failure, "recovery": recovery}

if SCENARIO not in SCENARIOS:
    fail(f"未知场景 {SCENARIO}")

root, data = prepare()
try:
    summary = SCENARIOS[SCENARIO](root, data)
finally:
    # 只清理本次自己建的临时目录。
    shutil.rmtree(root, ignore_errors=True)
print(f"{SCENARIO} 契约通过：{summary}")
