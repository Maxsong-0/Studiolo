#!/usr/bin/env python3
"""Assemble a credential-free test runtime as maxsong; never install or use sudo."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

CODEX_ARCHIVE_SHA = "a65b895c6ac1a73629bbe4b864640c86133e94a43b4d67b3103044e1a306d5a2"
CLAUDE_SHA = "6c8818fa22187aa555c242be4abbacc44d6b71a32ac9631ee7b2b5d12f51f752"
CLAUDE_SIGNER = "31DDDE24DDFAB679F42D7BD2BAA929FF1A7ECACE"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def run(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT)


def main():
    if os.geteuid() == 0 or os.uname().machine != "x86_64":
        raise SystemExit("Use the unprivileged x86_64 test host account")
    stage, binaries = [Path(value).resolve(strict=True) for value in sys.argv[1:]]
    downloads = stage / "downloads"
    assert digest(downloads / "codex-package.tar.gz") == CODEX_ARCHIVE_SHA
    assert digest(downloads / "claude") == CLAUDE_SHA
    manifest = json.loads((downloads / "manifest.json").read_text())
    assert manifest["version"] == "2.1.236"
    assert manifest["platforms"]["linux-x64"]["checksum"] == CLAUDE_SHA
    with tempfile.TemporaryDirectory(prefix="claude-signature-", dir=stage) as keyring:
        key = run("gpg", "--batch", "--homedir", keyring, "--with-colons",
                  "--import-options", "show-only", "--import", str(downloads / "claude-code.asc"))
        fingerprints = [line.split(":")[9] for line in key.splitlines() if line.startswith("fpr:")]
        assert fingerprints[0] == CLAUDE_SIGNER
        run("gpg", "--batch", "--homedir", keyring, "--import", str(downloads / "claude-code.asc"))
        signature = run("gpg", "--batch", "--homedir", keyring, "--status-fd", "1", "--verify",
                        str(downloads / "manifest.json.sig"), str(downloads / "manifest.json"))
        assert any(line.startswith("[GNUPG:] VALIDSIG ") and CLAUDE_SIGNER in line
                   for line in signature.splitlines())
    runtime = stage / "runtime"
    runtime.mkdir(mode=0o755)  # create-only: do not silently replace a prior assembly
    for name in ("usr/local", "usr/bin", "usr/lib/studiolo", "bin", "etc/ssl/certs",
                 "work", "inputs", "run", "home", "tmp", "proc", "dev", "sys/fs/cgroup"):
        (runtime / name).mkdir(parents=True, exist_ok=True)
    with tarfile.open(downloads / "codex-package.tar.gz") as archive:
        archive.extractall(runtime / "usr/local", filter="data")

    def copy(source, destination):
        target = runtime / destination.lstrip("/")
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target, follow_symlinks=True)
        target.chmod(0o755 if os.access(source, os.X_OK) else 0o644)

    copy(downloads / "claude", "/usr/local/bin/claude")
    (runtime / "usr/local/bin/claude").chmod(0o755)
    copy(runtime / "usr/local/codex-path/rg", "/usr/bin/rg")
    scripts = Path(__file__).resolve().parent
    for name in ("launch.py", "rg_probe.py"):
        copy(scripts / name, "/usr/lib/studiolo/" + name)
    tools = ("python3", "bash", "sh", "git", "cat", "cp", "mv", "rm", "mkdir", "touch",
             "find", "sed", "awk", "head", "tail", "wc", "sort", "diff", "patch", "timeout",
             "sha256sum", "env", "uname", "ls", "pwd", "printf", "true", "false")
    for name in tools:
        source = shutil.which(name, path="/usr/bin:/bin")
        assert source, name
        copy(source, "/usr/bin/" + name)
    for name in ("sh", "bash"):
        copy("/bin/" + name, "/bin/" + name)
    stdlib = Path("/usr/lib") / f"python{sys.version_info.major}.{sys.version_info.minor}"
    shutil.copytree(stdlib, runtime / stdlib.relative_to("/"), symlinks=False,
                    ignore=shutil.ignore_patterns("__pycache__", "test", "tests", "site-packages", "dist-packages"))
    copy("/etc/ssl/certs/ca-certificates.crt", "/etc/ssl/certs/ca-certificates.crt")
    (runtime / "etc/passwd").write_text("runner:x:65534:65534:Runner:/home/runner:/bin/bash\n")
    (runtime / "etc/group").write_text("runner:x:65534:\n")

    # Inspect only verified vendor ELFs and trusted system programs/extensions.
    # Optional bundled voice assets stay in their original package layout; they
    # are not the text-only execution profile and are not claimed to be usable.
    candidates = list((runtime / "usr/bin").glob("*")) + list((runtime / "bin").glob("*"))
    candidates += list((runtime / stdlib.relative_to("/")).rglob("*.so"))
    candidates += [runtime / "usr/local/bin" / name for name in ("claude", "codex", "codex-code-mode-host")]
    dependencies = set()
    for binary in candidates:
        with binary.open("rb") as stream:
            magic = stream.read(4)
        if magic != b"\x7fELF":
            continue
        dynamic = run("readelf", "-d", str(binary))
        if "(NEEDED)" not in dynamic:
            continue
        listing = run("ldd", str(binary))
        assert "not found" not in listing, (str(binary), listing)
        dependencies.update(re.findall(r"(?:=>\s+|^\s*)(/[^\s()]+)", listing, re.MULTILINE))
    for library in sorted(dependencies):
        resolved = Path(library).resolve(strict=True)
        assert resolved.is_relative_to("/usr/lib") and resolved.stat().st_uid == 0, library
        copy(library, library)
    # Remove package links by materializing their already checked in-tree target.
    for path in list(runtime.rglob("*")):
        if path.is_symlink():
            target = path.resolve(strict=True)
            assert target.is_relative_to(runtime) and target.is_file(), str(path)
            data, mode = target.read_bytes(), target.stat().st_mode & 0o777
            path.unlink()
            path.write_bytes(data)
            path.chmod(mode)
    for path in runtime.rglob("*"):
        path.chmod(0o755 if path.is_dir() or path.stat().st_mode & 0o111 else 0o644)
    (stage / "bin").mkdir()
    shutil.copyfile(binaries / "examples/probe", stage / "bin/probe")
    shutil.copyfile(binaries / "gateway-model-relay", stage / "bin/gateway-model-relay")
    for path in (stage / "bin").iterdir():
        path.chmod(0o755)
    hashes = {str(path.relative_to(stage)): digest(path)
              for root in (runtime, stage / "bin") for path in root.rglob("*") if path.is_file()}
    (stage / "files.json").write_text(json.dumps(hashes, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"files": len(hashes), "claude_sha256": digest(runtime / "usr/local/bin/claude"),
                      "codex_sha256": digest(runtime / "usr/local/bin/codex"),
                      "claude_manifest_signature": "verified", "system_libraries": len(dependencies)}))


if __name__ == "__main__":
    main()
