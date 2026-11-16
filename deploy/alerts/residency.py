"""Python 运维入口的 Linux 进程保护；措施和回执对应 process-safety，失败即停止。"""
import ctypes
import errno
import os
import resource
import sys


def harden():
    if sys.platform != "linux" or os.getuid() != os.geteuid() or os.getgid() != os.getegid():
        raise RuntimeError("residency_unavailable")
    if not (sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode):
        raise RuntimeError("isolated_python_required")
    os.umask(0o077)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    if resource.getrlimit(resource.RLIMIT_CORE) != (0, 0):
        raise RuntimeError("residency_unavailable")
    libc = ctypes.CDLL(None, use_errno=True)
    libc.prctl.argtypes = [ctypes.c_int] + [ctypes.c_ulong] * 4
    libc.prctl.restype = ctypes.c_int
    libc.mlockall.argtypes = [ctypes.c_int]
    libc.mlockall.restype = ctypes.c_int
    if libc.prctl(4, 0, 0, 0, 0) != 0 or libc.prctl(3, 0, 0, 0, 0) != 0:
        raise RuntimeError("residency_unavailable")
    memory = "locked_on_fault"
    if libc.mlockall(1 | 2 | 4) != 0:
        if ctypes.get_errno() != errno.EINVAL or libc.mlockall(1 | 2) != 0:
            raise RuntimeError("residency_unavailable")
        memory = "locked_eagerly"
    # 环境块没有擦除；只声明经过 PR_GET_DUMPABLE 实测的同 UID 外读屏障。
    print("residency: core_zero; not_dumpable; " + memory + "; environment_shielded_not_erased", file=sys.stderr, flush=True)


def child_environment():
    # ssh、systemctl 与 timedatectl 不继承告警 token；仅发送器继承完整环境。
    return {key: value for key, value in os.environ.items() if key in ("PATH", "LANG", "HOME")}
