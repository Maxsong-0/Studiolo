#!/usr/bin/env python3
"""仅 root/Linux/Python -I -B -S 的 portable 因子密文管线；Python 不读种子流。"""
import argparse
import contextlib
import os
import re
import resource
import stat
import subprocess
import sys
import time
import types


def load_security():
    """只从入口自身固定 root 目录加载配套代码，不使用搜索路径。"""
    path = os.path.abspath(__file__)
    parent = os.path.dirname(path)
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    fd = os.open("/", flags)
    try:
        parts = parent.split("/")[1:]
        for index, part in enumerate(parts):
            child = os.open(part, flags, dir_fd=fd)
            os.close(fd)
            fd = child
            item = os.fstat(fd)
            mode = stat.S_IMODE(item.st_mode)
            if item.st_uid != 0 or mode & 0o6000 or (mode & 0o022 and not (index < len(parts) - 1 and mode & stat.S_ISVTX)):
                raise ValueError("untrusted_tool")
        for name in (os.path.basename(path), "portable_factor_security.py"):
            source = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC, dir_fd=fd)
            try:
                before = os.fstat(source)
                if (not stat.S_ISREG(before.st_mode) or before.st_uid != 0 or before.st_nlink != 1
                        or before.st_mode & 0o7022 or before.st_size > 65536):
                    raise ValueError("untrusted_tool")
                with os.fdopen(os.dup(source), "rb") as stream:
                    raw = stream.read(65537)
                def identity(item):
                    return (item.st_dev, item.st_ino, item.st_mode, item.st_uid, item.st_gid,
                            item.st_nlink, item.st_size, item.st_mtime_ns, item.st_ctime_ns)
                if len(raw) != before.st_size or identity(before) != identity(os.fstat(source)) or identity(before) != identity(os.stat(name, dir_fd=fd, follow_symlinks=False)):
                    raise ValueError("changed_tool")
                if name == "portable_factor_security.py":
                    module = types.ModuleType("portable_factor_security")
                    module.__file__ = parent + "/" + name
                    exec(compile(raw, module.__file__, "exec"), module.__dict__)
                    return module
            finally:
                os.close(source)
    finally:
        os.close(fd)


SYSTEMD_CREDS = "/usr/bin/systemd-creds"
DEFAULT_HOST_KEY = "/var/lib/systemd/credential.secret"
TIMEOUT = 30
ENV = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"}


def child_limits():
    resource.setrlimit(resource.RLIMIT_FSIZE, (65536, 65536))


def terminate(children):
    for child in children:
        if child.poll() is None:
            child.kill()
    for child in children:
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            pass  # 内核不可中断状态不假报已回收；父任务失败，不发布密文。


def spawn(tool, argv, input_fd, output_fd, children, extra=(), environment=None):
    child = subprocess.Popen([argv[0], *argv[1:]], executable=f"/proc/self/fd/{tool[0]}",
        stdin=input_fd, stdout=output_fd, stderr=subprocess.DEVNULL, close_fds=True,
        pass_fds=(tool[0], *extra), env=environment or ENV, start_new_session=True,
        preexec_fn=child_limits)
    children.append(child)
    return child


def systemd_version(tool):
    # 此唯一捕获的是无凭据参数的固定版本元数据，绝不捕获 decrypt/encrypt 输出。
    result = subprocess.run([SYSTEMD_CREDS, "--version"], executable=f"/proc/self/fd/{tool[0]}",
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        close_fds=True, pass_fds=(tool[0],), env=ENV, timeout=3)
    first = result.stdout.split(b"\n", 1)[0]
    match = re.fullmatch(rb"systemd ([0-9]+)(?:\.[0-9]+)*(?: .*)?", first)
    if result.returncode != 0 or len(result.stdout) > 4096 or not match or int(match[1]) < 259:
        raise ValueError("unsupported_systemd")


def pipeline(args, security):
    security.protect()
    importing = args.command == "import"
    if importing:
        security.require(args.host_key is not None and stat.S_ISFIFO(os.fstat(0).st_mode) and not os.isatty(0))
    else:
        security.require(re.fullmatch(r"age1[023456789acdefghjklmnpqrstuvwxyz]{58}", args.recipient) is not None)
    with contextlib.ExitStack() as stack:
        filter_bin = security.opened(args.filter_bin, stack, executable=True, limit=64 * 1024**2)
        systemd = security.opened(SYSTEMD_CREDS, stack, executable=True, limit=64 * 1024**2)
        systemd_version(systemd)
        output = security.Output(args.output, stack)
        key_path = args.host_key or DEFAULT_HOST_KEY
        key_before = security.host_key(key_path, output, stack, importing)
        credential = None if importing else security.opened(args.credential, stack, mode=0o600)
        age = None if importing else security.opened(args.age_bin, stack, executable=True, limit=64 * 1024**2)
        creds_env = dict(ENV, SYSTEMD_CREDENTIAL_SECRET=key_path) if args.host_key else ENV
        children, pipes = [], []
        try:
            r1, w1 = os.pipe2(os.O_CLOEXEC)
            pipes += [r1, w1]
            if importing:
                spawn(filter_bin, [args.filter_bin], 0, w1, children)
                spawn(systemd, [SYSTEMD_CREDS, "encrypt", "--name=totp-next", "--with-key=host", "-", "-"],
                      r1, output.fd, children, environment=creds_env)
            else:
                r2, w2 = os.pipe2(os.O_CLOEXEC)
                pipes += [r2, w2]
                spawn(systemd, [SYSTEMD_CREDS, "decrypt", "--name=totp-next", "--newline=no",
                               f"/proc/self/fd/{credential[0]}", "-"],
                      subprocess.DEVNULL, w1, children, extra=(credential[0],), environment=creds_env)
                spawn(filter_bin, [args.filter_bin], r1, w2, children)
                spawn(age, [args.age_bin, "--encrypt", "-r", args.recipient], r2, output.fd, children)
            for fd in pipes:
                os.close(fd)
            pipes.clear()
            deadline = time.monotonic() + TIMEOUT
            while True:
                states = [child.poll() for child in children]
                security.require(all(code in (None, 0) for code in states))
                if all(code == 0 for code in states):
                    break
                security.require(time.monotonic() < deadline)
                time.sleep(0.02)
            for child in children:
                child.wait(timeout=1)
            for binding in (filter_bin, systemd, age, credential, key_before):
                if binding is not None:
                    security.bound_file(binding, stack)
            security.opened(key_path, stack, mode=0o400, limit=16384)
            output.publish(stack)
        finally:
            for fd in pipes:
                os.close(fd)
            terminate(children)


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ValueError("invalid_arguments")


def main():
    security = load_security()
    security.protect()
    parser = Parser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    export = commands.add_parser("export")
    for name in ("credential", "recipient", "output", "filter-bin"):
        export.add_argument("--" + name, required=True)
    export.add_argument("--age-bin", default="/usr/bin/age")
    export.add_argument("--host-key")
    importing = commands.add_parser("import")
    for name in ("output", "filter-bin", "host-key"):
        importing.add_argument("--" + name, required=True)
    args = parser.parse_args()
    pipeline(args, security)
    print("portable_factor_complete")


if __name__ == "__main__":
    try:
        main()
    except BaseException as error:
        if isinstance(error, SystemExit) and error.code == 0:
            raise
        os.write(2, b"portable_factor_failed\n")
        raise SystemExit(1) from None
