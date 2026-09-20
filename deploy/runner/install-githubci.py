#!/usr/bin/env python3
"""Explicit, create-only privileged installation for the githubci M6 test area.

Run from the user's SSH terminal with sudo. Does not read keys, launch models,
enable Gateway, alter CI services, relax AppArmor, or change sudo permissions.
"""
import hashlib
import json
import os
from pathlib import Path
import pwd
import shutil
import socket
import stat
import subprocess
import sys
import tempfile

STAGE = Path("/home/maxsong/.cache/studiolo-m6-setup/20260920")
INSTALL = Path("/opt/studiolo-m6-test")
DATA = Path("/var/lib/studiolo-m6-test")
CONFIG = Path("/etc/studiolo-m6-test")
GATEWAY = "studiolo-m6-gateway"


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def write(path, text, mode=0o644, owner=None):
    with path.open("x") as output:
        output.write(text)
    path.chmod(mode)
    if owner:
        os.chown(path, *owner)


def main():
    if sys.argv[1:] == ["--plan"]:
        print(json.dumps({"host": "githubci", "stage": str(STAGE), "install": str(INSTALL),
                          "data": str(DATA), "config": str(CONFIG), "new_account": GATEWAY,
                          "install_package": "bubblewrap", "start_gateway": False,
                          "read_credentials": False}, indent=2))
        return
    if sys.argv[1:] or os.geteuid() != 0 or os.environ.get("SUDO_USER") != "maxsong":
        raise SystemExit("Run sudo python3 install-githubci.py from maxsong's own terminal")
    os.environ["PATH"] = "/usr/sbin:/usr/bin:/sbin:/bin"
    os.umask(0o077)
    assert socket.gethostname() == "githubci" and os.uname().machine == "x86_64"
    front = pwd.getpwnam("maxsong")
    assert front.pw_uid == 1000
    assert not any(path.exists() or path.is_symlink() for path in (INSTALL, DATA, CONFIG))
    try:
        pwd.getpwnam(GATEWAY)
    except KeyError:
        pass
    else:
        raise SystemExit("Test account already exists; inspect the previous install before retrying")
    assert STAGE.resolve(strict=True) == STAGE and STAGE.stat().st_uid == front.pw_uid
    assert STAGE.stat().st_mode & 0o077 == 0
    inventory = json.loads((STAGE / "files.json").read_text())
    assert 5 <= len(inventory) <= 20000
    units = Path("/etc/systemd/system")
    mount_units = {name: run("systemd-escape", "--path", "--suffix=mount", str(DATA / name))
                   for name in ("attempts", "raw")}
    names = list(mount_units.values()) + ["studiolo-m6-keyholder.service",
                                        "studiolo-m6-raw-clean.service", "studiolo-m6-raw-clean.timer"]
    assert not any((units / name).exists() or (units / name).is_symlink() for name in names)
    # Copy into root-private staging first, then validate exactly that snapshot.
    # Links, devices, setuid bits and unlisted files are never installed.
    with tempfile.TemporaryDirectory(prefix="studiolo-m6-root-") as temporary:
        snapshot = Path(temporary)
        for relative, expected in inventory.items():
            name = Path(relative)
            assert not name.is_absolute() and ".." not in name.parts and name.parts[0] in ("runtime", "bin")
            source = STAGE / name
            assert source.resolve(strict=True) == source
            with source.open("rb") as stream:
                metadata = os.fstat(stream.fileno())
                assert stat.S_ISREG(metadata.st_mode) and metadata.st_uid == front.pw_uid
                destination = snapshot / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                with destination.open("xb") as output:
                    shutil.copyfileobj(stream, output)
            with destination.open("rb") as stream:
                assert hashlib.file_digest(stream, "sha256").hexdigest() == expected, relative
            destination.chmod(0o755 if metadata.st_mode & 0o111 else 0o644)
        for name in ("work", "inputs", "run", "home", "tmp", "proc", "dev", "sys/fs/cgroup"):
            (snapshot / "runtime" / name).mkdir(parents=True, exist_ok=True)
        subprocess.run(["apt-get", "install", "--yes", "--no-install-recommends", "bubblewrap"], check=True)
        subprocess.run(["useradd", "--system", "--user-group", "--no-create-home", "--home-dir",
                        "/nonexistent", "--shell", "/usr/sbin/nologin", GATEWAY], check=True)
        keyholder = pwd.getpwnam(GATEWAY)
        shutil.copytree(snapshot, INSTALL)
    for path in [INSTALL] + [p for p in INSTALL.rglob("*") if p.is_dir()]:
        path.chmod(0o755)
    DATA.mkdir(mode=0o755)
    DATA.chmod(0o755)
    for name in ("attempts", "raw", "retained", "state", "transport"):
        path = DATA / name
        path.mkdir(mode=0o700)
        os.chown(path, front.pw_uid, front.pw_gid)
    credentials = DATA / "credentials"
    credentials.mkdir(mode=0o700)
    os.chown(credentials, keyholder.pw_uid, keyholder.pw_gid)
    CONFIG.mkdir(mode=0o755)
    CONFIG.chmod(0o755)
    config = {"schema": 1, "socket": "/run/studiolo-m6-keyholder/model.sock", "client_uid": front.pw_uid,
              "anthropic_key_file": str(credentials / "anthropic"), "openai_key_file": str(credentials / "openai")}
    write(CONFIG / "gateway.json", json.dumps(config, indent=2) + "\n", 0o600,
          (keyholder.pw_uid, keyholder.pw_gid))
    write(INSTALL / "files.json", json.dumps(inventory, indent=2, sort_keys=True) + "\n")
    for name, size in (("attempts", 512), ("raw", 128)):
        execution = ",noexec" if name == "raw" else ""
        write(units / mount_units[name], f"""[Unit]
Description=Studiolo M6 test {name} bounded tmpfs
[Mount]
What=tmpfs
Where={DATA / name}
Type=tmpfs
Options=size={size}M,mode=0700,uid={front.pw_uid},gid={front.pw_gid},nosuid,nodev,noswap{execution}
[Install]
WantedBy=multi-user.target
""")
    write(units / "studiolo-m6-keyholder.service", f"""[Unit]
Description=Studiolo M6 dedicated TEST API key-holder (manual start only)
ConditionPathExists={credentials / 'anthropic'}
ConditionPathExists={credentials / 'openai'}
[Service]
Type=simple
User={GATEWAY}
Group={front.pw_name}
ExecStart={INSTALL}/bin/gateway-model-relay {CONFIG}/gateway.json
RuntimeDirectory=studiolo-m6-keyholder
RuntimeDirectoryMode=0750
UMask=0077
LimitCORE=0
LimitMEMLOCK=infinity
MemoryMax=1G
TasksMax=64
NoNewPrivileges=true
PrivateTmp=true
PrivateDevices=true
ProtectHome=true
ProtectSystem=strict
ProtectKernelTunables=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
InaccessiblePaths=-/home/ghrunner -{DATA}/state -{DATA}/retained
""")
    # Match the application's filename deadline, independent of Gateway uptime.
    write(INSTALL / "raw-clean.py", '''import os, pathlib, stat, time
root = pathlib.Path("/var/lib/studiolo-m6-test/raw")
assert root.is_mount()
now = int(time.time())
for entry in os.scandir(root):
    if not entry.name.endswith(".raw"):
        continue
    deadline = int(entry.name[:-4].rsplit("-", 1)[1])
    assert stat.S_ISREG(entry.stat(follow_symlinks=False).st_mode)
    if now >= deadline:
        os.unlink(entry.path)
''')
    write(units / "studiolo-m6-raw-clean.service", f"""[Unit]
Description=Studiolo M6 raw deadline cleanup independent of Gateway
Requires={mount_units['raw']}
After={mount_units['raw']}
[Service]
Type=oneshot
User={front.pw_name}
ExecStart=/usr/bin/python3 {INSTALL}/raw-clean.py
NoNewPrivileges=true
ProtectHome=true
ProtectSystem=strict
ReadWritePaths={DATA}/raw
""")
    write(units / "studiolo-m6-raw-clean.timer", """[Unit]
Description=Studiolo M6 raw deadline checks
[Timer]
OnBootSec=30s
OnUnitActiveSec=30s
AccuracySec=1s
[Install]
WantedBy=timers.target
""")
    subprocess.run(["systemd-analyze", "verify", *[str(units / name) for name in names]], check=True)
    subprocess.run(["systemctl", "daemon-reload"], check=True)
    subprocess.run(["systemctl", "enable", "--now", *mount_units.values(),
                    "studiolo-m6-raw-clean.timer"], check=True)
    print("M6 test runtime installed. No keys read; Gateway remains stopped. Ready for real containment probes.")


if __name__ == "__main__":
    main()
