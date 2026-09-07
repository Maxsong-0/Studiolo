"""只使用独立临时双库、虚构RFC测试种子；生产二进制不启动Python。"""

import errno
import hashlib
import hmac
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time

BINARY = str(Path(sys.argv[1]).resolve())
SEED = b"12345678901234567890"
VARIABLE = "STUDIOLO_TOTP_SEED_HEX"
PROMPT = "请输入六位验证码".encode()
ENVIRONMENT = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C.UTF-8"}


def otp():
    digest = hmac.new(SEED, struct.pack(">Q", int(time.time()) // 30), hashlib.sha1).digest()
    offset = digest[-1] & 15
    value = struct.unpack(">I", digest[offset:offset + 4])[0] & 0x7fffffff
    return f"{value % 1000000:06d}"


def invoke(arguments, code=None, seed=SEED.hex(), broken_stdout=False):
    environment = ENVIRONMENT.copy()
    if seed is not None:
        environment[VARIABLE] = seed
    ready_read, ready_write = os.pipe()
    pid, master = pty.fork()
    if pid == 0:
        os.close(ready_write)
        os.read(ready_read, 1)
        os.close(ready_read)
        if broken_stdout:
            closed_read, output_write = os.pipe()
            os.close(closed_read)
            os.dup2(output_write, 1)
            os.close(output_write)
        os.execve(BINARY, [BINARY, *arguments], environment)
    os.close(ready_read)
    output = bytearray()
    sent = False
    status = None
    original = termios.tcgetattr(master)
    os.write(ready_write, b"1")
    os.close(ready_write)
    deadline = time.monotonic() + 15
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.05)[0]:
                try:
                    chunk = os.read(master, 4096)
                    if chunk:
                        output.extend(chunk)
                except OSError as error:
                    if error.errno != errno.EIO:
                        raise
            if code is not None and not sent and PROMPT in output:
                assert not termios.tcgetattr(master)[3] & termios.ECHO
                os.write(master, code.encode() + b"\n")
                sent = True
            done, value = os.waitpid(pid, os.WNOHANG)
            if done:
                status = value
                break
        assert status is not None, "实际CLI执行超时"
        # 子进程退出后继续读取最后的固定结果，不把未读到输出当作无泄漏。
        while select.select([master], [], [], 0.02)[0]:
            try:
                chunk = os.read(master, 4096)
                if not chunk:
                    break
                output.extend(chunk)
            except OSError as error:
                if error.errno == errno.EIO:
                    break
                raise
        assert termios.tcgetattr(master) == original, "实际CLI未恢复完整终端属性"
        assert SEED.hex().encode() not in output
        if code is not None:
            assert code.encode() not in output, "验证码出现在终端输出"
        return os.waitstatus_to_exitcode(status), bytes(output), sent
    finally:
        if status is None:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        os.close(master)


with tempfile.TemporaryDirectory(prefix="studiolo-auth-admin-process-") as temporary:
    root = Path(temporary).resolve()
    root.chmod(0o700)
    auth, audit = root / "auth", root / "audit"
    auth.mkdir(mode=0o700)
    audit.mkdir(mode=0o700)
    config = root / "owner.json"
    config.write_text(json.dumps({
        "version": 1, "auth_directory": str(auth), "audit_directory": str(audit),
        "owner": {"issuer": "https://fixture.cloudflareaccess.com", "audience": "fictional",
                  "subject": "fixture-owner", "email": "owner@example.test"},
        "idle_seconds": 1800, "absolute_seconds": 43200, "seed_ref": "fixture-totp-v1",
    }))
    config.chmod(0o600)
    enroll = ["enroll", "--config", str(config)]
    migrate = ["migrate", "--config", str(config), "--from", "3"]

    # 管道输入不能借同一进程环境登记，更不能创建数据库。
    pipe = subprocess.run([BINARY, *enroll], input=b"123456\n", capture_output=True,
                          env={**ENVIRONMENT, VARIABLE: SEED.hex()}, timeout=15)
    assert pipe.returncode != 0 and b"123456" not in pipe.stdout + pipe.stderr
    assert not list(auth.iterdir()) and not list(audit.iterdir())
    assert invoke(enroll)[0] != 0

    assert not list(auth.iterdir()) and not list(audit.iterdir())

    # 明确初始化不读取秘密；缺失/畸形环境在数据库装配前拒绝。
    assert invoke(migrate, seed=None)[0] == 0
    for invalid in [None, "", "0" * 39, "A" * 40, "g" * 40]:
        result = invoke(enroll, seed=invalid)
        assert result[0] != 0 and PROMPT not in result[1]

    code = otp()
    result = invoke(enroll, code=code)
    assert result[0] == 0 and result[2] and "首次TOTP登记已提交".encode() in result[1], result
    again = invoke(enroll)
    assert again[0] != 0 and PROMPT not in again[1]
    assert "已经登记".encode() in again[1]
    assert invoke(["audit-flush", "--config", str(config)])[0] == 0
    assert invoke(migrate)[0] == 0
    assert invoke(enroll)[0] != 0

    # 已提交但结果输出失败不能伪装成回滚；再次调用不得重新索取OTP。
    second = json.loads(config.read_text())
    for key, name in [("auth_directory", "auth-second"), ("audit_directory", "audit-second")]:
        directory = root / name
        directory.mkdir(mode=0o700)
        second[key] = str(directory)
    config.write_text(json.dumps(second))
    assert invoke(migrate, seed=None)[0] == 0
    failed_output = invoke(enroll, code=otp(), broken_stdout=True)
    assert failed_output[0] != 0 and failed_output[2]
    assert "结果输出失败".encode() in failed_output[1], failed_output
    assert "禁止自动重试".encode() in failed_output[1]
    repeated = invoke(enroll)
    assert repeated[0] != 0 and PROMPT not in repeated[1]
    assert "已经登记".encode() in repeated[1]

    # 只有参数错误，不回显未知参数或提供秘密参数入口。
    malformed = subprocess.run([BINARY, "enroll", "--seed", "fictional-do-not-echo"],
                               capture_output=True, env=ENVIRONMENT, timeout=15)
    assert malformed.returncode != 0
    assert b"fictional-do-not-echo" not in malformed.stdout + malformed.stderr

print("实际CLI进程契约通过：无TTY拒绝、显式初始化、严格环境、首次登记、重复拒绝与秘密不回显")
