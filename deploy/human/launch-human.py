#!/usr/bin/python3
"""T-092: one credential to one fixed product exec; never a provisioning tool."""

import ctypes
import os
import pwd
import resource
import stat
import struct
import sys

RELEASE = "/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r2"
BINARY = RELEASE + "/bin/studiolo-local-server"
CONFIG = "/home/maxsong/.config/studiolo-human/owner.json"
AUTH = "/home/maxsong/.local/share/studiolo-human-auth"
AUDIT = "/home/maxsong/.local/share/studiolo-human-audit"
POLICY = "/home/maxsong/.local/share/studiolo-agent-policy"
SOCKET = "/run/studiolo-human/http.sock"
ORIGIN = "https://studiolo.maxsong.me"
CREDENTIALS = "/run/credentials/studiolo-human.service"


def require(condition):
    if not condition:
        raise RuntimeError("rejected")


def protect_process():
    require(sys.platform == "linux" and len(sys.argv) == 1)
    require(sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode)
    owner = pwd.getpwnam("maxsong")
    require(os.getuid() == os.geteuid() == owner.pw_uid != 0)
    require(os.getgid() == os.getegid() == owner.pw_gid)
    os.umask(0o077)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    require(resource.getrlimit(resource.RLIMIT_CORE) == (0, 0))
    libc = ctypes.CDLL(None, use_errno=True)
    libc.prctl.argtypes = [ctypes.c_int] + [ctypes.c_ulong] * 4
    libc.prctl.restype = ctypes.c_int
    require(libc.prctl(4, 0, 0, 0, 0) == 0)  # PR_SET_DUMPABLE
    require(libc.prctl(3, 0, 0, 0, 0) == 0)  # PR_GET_DUMPABLE


def open_directory(path, private=False):
    require(isinstance(path, str) and path.startswith("/") and len(path) <= 4096)
    parts = path.split("/")[1:]
    require(parts and all(part not in ("", ".", "..") for part in parts))
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW | os.O_CLOEXEC
    current = os.open("/", flags)
    try:
        for part in parts:
            next_fd = os.open(part, flags, dir_fd=current)
            os.close(current)
            current = next_fd
            metadata = os.fstat(current)
            require(metadata.st_uid in (0, os.geteuid()))
            require(stat.S_IMODE(metadata.st_mode) & 0o7022 == 0)
        if private:
            require(stat.S_IMODE(metadata.st_mode) & 0o0077 == 0)
        return current
    except BaseException:
        os.close(current)
        raise


def inspect_existing(path, mode, private_parent=True):
    parent, name = path.rsplit("/", 1)
    parent_fd = open_directory(parent, private=private_parent)
    descriptor = None
    try:
        descriptor = os.open(
            name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
            dir_fd=parent_fd,
        )
        metadata = os.fstat(descriptor)
        require(stat.S_ISREG(metadata.st_mode) and metadata.st_nlink == 1)
        require(metadata.st_uid == os.geteuid())
        require(stat.S_IMODE(metadata.st_mode) == mode)
    finally:
        if descriptor is not None:
            os.close(descriptor)
        os.close(parent_fd)


def preflight():
    # Metadata only: refuse missing stores before PolicyStore::open can create one.
    inspect_existing(CONFIG, 0o600)
    for directory, name in ((AUTH, "auth"), (AUDIT, "audit"), (POLICY, "policy")):
        inspect_existing(directory + "/" + name + ".sqlite3", 0o600)
    inspect_existing(BINARY, 0o755, private_parent=False)
    for directory in (RELEASE + "/example-vault", RELEASE + "/web"):
        os.close(open_directory(directory))
    parent_fd = open_directory("/run/studiolo-human", private=True)
    try:
        try:
            os.stat("http.sock", dir_fd=parent_fd, follow_symlinks=False)
        except FileNotFoundError:
            pass
        else:
            raise RuntimeError("rejected")
    finally:
        os.close(parent_fd)


def systemd_credential_metadata(descriptor, directory=False):
    """Accept only systemd's root-owned ACL for this service UID, not group access."""
    metadata = os.fstat(descriptor)
    permission = 0o5 if directory else 0o4
    require(metadata.st_uid == metadata.st_gid == 0)
    require(stat.S_IMODE(metadata.st_mode) == (0o550 if directory else 0o440))
    require(stat.S_ISDIR(metadata.st_mode) if directory else stat.S_ISREG(metadata.st_mode))
    if not directory:
        require(metadata.st_nlink == 1 and metadata.st_size == 40)
    # Linux POSIX ACL xattr v2: owner, this named user, empty owning group,
    # matching read(/execute) mask, empty other. Exact bytes reject extra grants.
    undefined = 0xFFFFFFFF
    entries = ((0x01, permission, undefined), (0x02, permission, os.geteuid()),
               (0x04, 0, undefined), (0x10, permission, undefined),
               (0x20, 0, undefined))
    expected = struct.pack("<I", 2) + b"".join(struct.pack("<HHI", *e) for e in entries)
    require(os.getxattr(descriptor, "system.posix_acl_access") == expected)
    return metadata


def credential():
    require(os.environ.get("CREDENTIALS_DIRECTORY") == CREDENTIALS)
    # Only this fixed systemd directory uses the dedicated ACL contract;
    # owner.json, database and socket private-directory checks remain unchanged.
    parent_fd = open_directory(CREDENTIALS)
    descriptor = None
    try:
        systemd_credential_metadata(parent_fd, directory=True)
        descriptor = os.open(
            "totp-seed", os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC,
            dir_fd=parent_fd,
        )
        before = systemd_credential_metadata(descriptor)
        value = os.read(descriptor, 41)
        after = systemd_credential_metadata(descriptor)
        systemd_credential_metadata(parent_fd, directory=True)
        require((before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns,
                 before.st_ctime_ns) == (after.st_dev, after.st_ino, after.st_size,
                                        after.st_mtime_ns, after.st_ctime_ns))
        require(len(value) == 40 and all(c in b"0123456789abcdef" for c in value))
        return value.decode("ascii")
    finally:
        if descriptor is not None:
            os.close(descriptor)
        os.close(parent_fd)


def main():
    protect_process()
    preflight()
    require("STUDIOLO_TOTP_SEED_HEX" not in os.environ)
    require("STUDIOLO_TOTP_REPLACEMENT_SEED_HEX" not in os.environ)
    arguments = [
        BINARY, "--vault", RELEASE + "/example-vault", "--assets", RELEASE + "/web",
        "--label", "服务器示例库", "--session-config", CONFIG,
        "--public-origin", ORIGIN, "--factor-socket", SOCKET, "--policy-data", POLICY,
    ]
    # No shell, child process, secret argv, inherited credential path, or .env file.
    environment = {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8",
                   "STUDIOLO_TOTP_SEED_HEX": credential()}
    os.execve(BINARY, arguments, environment)


if __name__ == "__main__":
    try:
        main()
    except BaseException:
        # Python buffers are not claimed to be zeroized; terminate without traceback.
        os.write(2, b"studiolo-human: startup rejected\n")
        os._exit(1)
