#!/usr/bin/env python3
"""真实控制 PTY 契约；参数是 auth-admin 的 Rust test executable，不执行真实登记。"""

import errno
import os
import pty
import select
import signal
import sys
import time

PROMPT = "请输入六位验证码（不回显）：".encode()
LIMIT = 65536


def run_case(executable, name, sent=b"", expected=b"Ok(())", mode="read", redirect=None):
    captured_read, captured_write = os.pipe()
    pid, master = pty.fork()
    if pid == 0:
        os.close(captured_read)
        if redirect == "stdin":
            read_fd, write_fd = os.pipe()
            os.write(write_fd, b"482719\n")
            os.close(write_fd)
            os.dup2(read_fd, 0)
            os.close(read_fd)
        if redirect == "stderr":
            with open(os.devnull, "wb", buffering=0) as null:
                os.dup2(null.fileno(), 2)
        if redirect == "no_control":
            # 新子进程保留真实TTY描述符，但setsid后没有控制终端；不依赖平台TIOCNOTTY差异。
            detached = os.fork()
            if detached:
                _, detached_status = os.waitpid(detached, 0)
                os._exit(os.waitstatus_to_exitcode(detached_status))
            os.setsid()
        if redirect == "foreign_tty":
            foreign_master, foreign_slave = pty.openpty()
            os.dup2(foreign_slave, 0)
            os.close(foreign_slave)
            os.set_inheritable(foreign_master, True)
        # 标记走独立stdout管道；关闭PTY master后仍可核对HUP的错误返回。
        os.dup2(captured_write, 1)
        os.close(captured_write)
        environment = {"PATH": os.defpath, "STUDIOLO_TERMINAL_PROBE": mode}
        os.execve(executable, [executable, "--exact", "terminal::tests::terminal_probe", "--nocapture", "--test-threads=1"], environment)

    os.close(captured_write)
    data = bytearray()
    terminal = bytearray()
    action_sent = False
    after_sent = False
    closed_master = False
    status = None
    active = [master, captured_read]
    deadline = time.monotonic() + 8
    try:
        while time.monotonic() < deadline:
            ready, _, _ = select.select(active, [], [], 0.1)
            for fd in ready:
                try:
                    chunk = os.read(fd, 4096)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
                    chunk = b""
                if not chunk:
                    active.remove(fd)
                    continue
                (terminal if fd == master else data).extend(chunk)
                assert len(data) + len(terminal) <= LIMIT, f"{name}: 输出超出预算"
            if mode != "preflight" and not action_sent and PROMPT in terminal:
                action_sent = True
                if name == "hangup":
                    os.close(master)
                    closed_master = True
                    if master in active:
                        active.remove(master)
                elif name == "external_sigint":
                    os.kill(pid, signal.SIGINT)
                else:
                    os.write(master, sent)
            if mode == "after_signal" and not after_sent and b"POST_READ_READY" in data:
                after_sent = True
                os.kill(pid, signal.SIGHUP if name == "after_hangup" else signal.SIGINT)
            if status is None:
                done, value = os.waitpid(pid, os.WNOHANG)
                if done:
                    status = value
            if status is not None and not active:
                break
        # 子进程仅收到固定虚构码和净化环境；保留探针状态，便于跨平台诊断。
        diagnostic = f"probe={bytes(data)!r}; terminal={bytes(terminal)!r}; status={status}"
        assert status is not None, f"{name}: 子进程未及时退出; {diagnostic}"
        assert mode == "preflight" or action_sent, f"{name}: 未出现隐藏输入提示; {diagnostic}"
        if name == "hangup":
            # Linux先唤醒slave读者再发送HUP，read可能先报EIO，此时恢复仍可成功。
            # 只额外接受已核对恢复的Io；取消/无法恢复仍按原契约处理，均不返回验证码。
            interrupted = any(b"PROBE_RESULT=" + value in data for value in [b"Err(Cancelled)", b"Err(Restore)"])
            restored_io = b"PROBE_RESULT=Err(Io);RESTORED=true" in data
            assert interrupted or restored_io, diagnostic
        else:
            assert b"PROBE_RESULT=" + expected in data, f"{name}: 错误返回与契约不符; {diagnostic}"
        if name != "hangup":
            assert b"RESTORED=true" in data, f"{name}: 终端属性未恢复; {diagnostic}"
        if mode == "after_signal":
            assert after_sent, f"{name}: 未触发读后信号; {diagnostic}"
            expected_signal = signal.SIGHUP if name == "after_hangup" else signal.SIGINT
            assert os.WIFSIGNALED(status) and os.WTERMSIG(status) == expected_signal, f"{name}: 默认信号行为未恢复; {diagnostic}"
        elif name == "hangup" and os.WIFSIGNALED(status):
            # 已核对错误返回之后，迟到HUP可按恢复的默认行为退出；不接受任意失败状态。
            assert os.WTERMSIG(status) == signal.SIGHUP, f"{name}: 非预期终止信号; {diagnostic}"
        else:
            assert os.WIFEXITED(status) and os.WEXITSTATUS(status) == 0, f"{name}: 探针失败; {diagnostic}"
        for secret in [b"482719", b"4827190", "４８２７１９".encode()]:
            assert secret not in terminal and secret not in data, f"{name}: 输入发生回显"
    finally:
        if status is None:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        if not closed_master:
            os.close(master)
        os.close(captured_read)


def main():
    if len(sys.argv) != 2:
        raise SystemExit("用法：python3 terminal_pty.py /absolute/auth-admin-test-executable")
    executable = os.path.abspath(sys.argv[1])
    cases = [
        ("valid", b"482719\n", b"Ok(())"),
        ("backspace", b"48271\x7f19\n", b"Ok(())"),
        ("short", b"48271\n", b"Err(InvalidCode)"),
        ("overlong", b"4827190", b"Err(InvalidCode)"),
        ("non_ascii", "４８２７１９".encode(), b"Err(InvalidCode)"),
        ("eof", b"\x04", b"Err(Eof)"),
        ("eof_after_six", b"482719\x04", b"Err(Eof)"),
        ("ctrl_c", b"\x03", b"Err(Cancelled)"),
        ("external_sigint", b"", b"Err(Cancelled)"),
        ("hangup", b"", b"Err(Restore)"),
    ]
    for name, sent, expected in cases:
        run_case(executable, name, sent, expected)
    for redirect in ["stdin", "stderr", "no_control", "foreign_tty"]:
        run_case(executable, "pipe_" + redirect, expected=b"Err(NotControlling)", mode="preflight", redirect=redirect)
    run_case(executable, "after_signal", b"482719\n", mode="after_signal")
    run_case(executable, "after_hangup", b"482719\n", mode="after_signal")
    print("控制TTY契约：16项通过")


if __name__ == "__main__":
    main()
