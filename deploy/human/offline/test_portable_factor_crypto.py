#!/usr/bin/env python3
"""仅在主线安排的隔离 root/Linux 环境运行，所有种子和密钥均为本次虚构。"""
import argparse
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest

OPTIONS = None
FIXTURE = b"0123456789abcdef" * 2 + b"01234567"
ENV = {"PATH": "/usr/bin:/bin", "LANG": "C", "LC_ALL": "C"}


def cleanup_child(child):
    if child.poll() is None:
        child.kill()
    child.communicate(timeout=3)


@unittest.skipUnless(sys.platform == "linux" and os.geteuid() == 0, "需要隔离 root/Linux 环境")
class CryptoTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode)
        parent = Path(OPTIONS.work_parent)
        self.assertTrue(parent.is_absolute() and stat.S_IMODE(parent.stat().st_mode) == 0o700)
        self.temporary = tempfile.TemporaryDirectory(prefix="portable-fictional-", dir=parent)
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.old_umask = os.umask(0o077)
        self.addCleanup(os.umask, self.old_umask)
        self.source, self.target = self.root / "source", self.root / "target"
        self.source.mkdir(mode=0o700)
        self.target.mkdir(mode=0o700)
        for name in ("portable_factor.py", "portable_factor_security.py"):
            shutil.copyfile(Path(__file__).with_name(name), self.root / name)
            (self.root / name).chmod(0o600)
        self.entry = self.root / "portable_factor.py"
        self.filter = self.root / "factor-pipe"
        shutil.copyfile(OPTIONS.filter_bin, self.filter)
        self.filter.chmod(0o700)
        self.source_key, self.target_key = self.source / "host-key", self.target / "host-key"
        self.credential, self.restored = self.source / "totp-next", self.target / "totp-next"
        self.identity = self.root / "identity.agekey"
        self.invoke([OPTIONS.age_keygen, "-o", str(self.identity)], output=subprocess.DEVNULL)
        self.recipient = self.invoke([OPTIONS.age_keygen, "-y", str(self.identity)]).stdout.decode().strip()
        with self.credential.open("xb") as ciphertext:
            self.invoke(["/usr/bin/systemd-creds", "encrypt", "--name=totp-next", "--with-key=host", "-", "-"],
                        data=FIXTURE, output=ciphertext, key=self.source_key)
        self.key_metadata(self.source_key)
        self.portable = self.root / "portable-factor.age"

    def invoke(self, argv, data=None, output=subprocess.PIPE, key=None, ok=True):
        environment = dict(ENV, SYSTEMD_CREDENTIAL_SECRET=str(key)) if key else ENV
        result = subprocess.run(argv, input=data, stdout=output, stderr=subprocess.PIPE,
                                env=environment, timeout=35)
        self.assertTrue((result.returncode == 0) == ok, "虚构子进程状态不符合预期")
        self.assertTrue(FIXTURE not in result.stderr, "虚构秘密回显到了错误通道")
        return result

    def wrapper(self, command, *arguments, data=None, ok=True):
        result = self.invoke([sys.executable, "-I", "-B", "-S", str(self.entry), command,
            "--filter-bin", str(self.filter), *map(str, arguments)], data=data, ok=ok)
        self.assertTrue(result.stdout == (b"portable_factor_complete\n" if ok else b""))
        if not ok:
            self.assertTrue(result.stderr == b"portable_factor_failed\n")
        return result

    def key_metadata(self, path):
        item = path.lstat()
        self.assertTrue(stat.S_ISREG(item.st_mode) and item.st_uid == 0 and item.st_nlink == 1)
        self.assertEqual(stat.S_IMODE(item.st_mode), 0o400)
        self.assertTrue(0 < item.st_size <= 16384)

    def export(self):
        self.wrapper("export", "--credential", self.credential, "--recipient", self.recipient,
            "--host-key", self.source_key, "--age-bin", OPTIONS.age_bin, "--output", self.portable)
        self.assertEqual(stat.S_IMODE(self.portable.stat().st_mode), 0o600)

    def restore(self):
        # age 先完整认证到 /dev/null；第二遍明文仅由两个子进程间的管道传递。
        age_command = [OPTIONS.age_bin, "--decrypt", "-i", str(self.identity), str(self.portable)]
        self.invoke(age_command, output=subprocess.DEVNULL)
        age = subprocess.Popen(age_command, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, env=ENV)
        importing = None
        try:
            importing = subprocess.Popen([sys.executable, "-I", "-B", "-S", str(self.entry), "import",
                "--output", str(self.restored), "--filter-bin", str(self.filter), "--host-key", str(self.target_key)],
                stdin=age.stdout, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=ENV)
            age.stdout.close()
            age.stdout = None
            stdout, stderr = importing.communicate(timeout=35)
            self.assertTrue(importing.returncode == 0 and stdout == b"portable_factor_complete\n" and stderr == b"")
            self.assertEqual(age.wait(timeout=3), 0)
        finally:
            if importing is not None:
                cleanup_child(importing)
            cleanup_child(age)
        self.key_metadata(self.target_key)

    def test_roundtrip_and_cross_host_key_rejection(self):
        before = self.source_key.stat()
        self.export()
        self.restore()
        old = self.invoke(["/usr/bin/systemd-creds", "decrypt", "--name=totp-next", "--newline=no",
            str(self.credential), "-"], output=subprocess.DEVNULL, key=self.target_key, ok=False)
        self.assertNotEqual(old.returncode, 0)
        restored = self.invoke(["/usr/bin/systemd-creds", "decrypt", "--name=totp-next", "--newline=no",
            str(self.restored), "-"], key=self.target_key)
        self.assertTrue(restored.stdout == FIXTURE)
        after = self.source_key.stat()
        self.assertEqual((before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns),
                         (after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns))
        self.assertFalse(list(self.root.rglob(".portable-factor-partial-*")))

    def test_invalid_seed_never_publishes(self):
        for index, data in enumerate((b"", FIXTURE[:-1], FIXTURE + b"0", FIXTURE + b"\n", b"A" + FIXTURE[1:])):
            with self.subTest(case=index):
                output = self.target / ("invalid-" + str(index))
                self.wrapper("import", "--output", output, "--host-key", self.target_key, data=data, ok=False)
                self.assertFalse(output.exists())

    def test_bad_host_key_mode_and_missing_host_key_reject(self):
        self.source_key.chmod(0o600)
        self.wrapper("export", "--credential", self.credential, "--recipient", self.recipient,
            "--host-key", self.source_key, "--age-bin", OPTIONS.age_bin, "--output", self.portable, ok=False)
        self.wrapper("import", "--output", self.restored, data=FIXTURE, ok=False)
        self.assertFalse(self.restored.exists() or self.portable.exists())

    def test_existing_output_not_overwritten(self):
        self.restored.write_bytes(b"fictional-sentinel")
        self.wrapper("import", "--output", self.restored, "--host-key", self.target_key, data=FIXTURE, ok=False)
        self.assertTrue(self.restored.read_bytes() == b"fictional-sentinel")
        self.assertFalse(self.target_key.exists())

    def test_age_authentication_rejects_truncated_ciphertext(self):
        self.export()
        with self.portable.open("r+b") as ciphertext:
            ciphertext.truncate(self.portable.stat().st_size - 1)
        self.invoke([OPTIONS.age_bin, "--decrypt", "-i", str(self.identity), str(self.portable)],
                    output=subprocess.DEVNULL, ok=False)
        self.assertFalse(self.restored.exists())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("filter-bin", "age-bin", "age-keygen", "work-parent"):
        parser.add_argument("--" + option, required=True)
    OPTIONS, remaining = parser.parse_known_args()
    unittest.main(argv=[sys.argv[0], *remaining])
