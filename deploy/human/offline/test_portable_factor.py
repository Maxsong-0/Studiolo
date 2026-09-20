#!/usr/bin/env python3
"""虚构过滤器及管线测试；真实 systemd/age 回路位于独立的可选测试入口。"""
import argparse
import contextlib
import os
from pathlib import Path
import resource
import runpy
import select
import shutil
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

FILTER = None
ROOT_FLOW = False
SOURCE = Path(__file__).with_name("factor_pipe.c")
FIXTURE = b"0123456789abcdef" * 2 + b"01234567"


def finish(child):
    if child.poll() is None:
        child.kill()
    child.communicate(timeout=3)


@unittest.skipUnless(sys.platform == "linux", "仅原生 Linux")
class FilterTests(unittest.TestCase):
    def call(self, data, ok=False, binary=None, preexec=None):
        result = subprocess.run([str(binary or FILTER)], input=data, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=18, preexec_fn=preexec)
        self.assertEqual(result.returncode == 0, ok)
        self.assertTrue(result.stdout == FIXTURE if ok else result.stdout == b"")
        self.assertNotIn(FIXTURE, result.stderr)
        if not ok:
            self.assertEqual(result.stderr, b"factor_pipe_failed\n")

    def test_exact_40_lowercase_hex(self):
        self.call(FIXTURE, ok=True)

    def test_bad_length_newline_hex_and_empty(self):
        for data in (b"", FIXTURE[:-1], FIXTURE + b"0", FIXTURE + b"\n", b"g" + FIXTURE[1:], b"A" + FIXTURE[1:]):
            with self.subTest(length=len(data)):
                self.call(data)

    def test_no_output_before_eof(self):
        child = subprocess.Popen([str(FILTER)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            child.stdin.write(FIXTURE)
            child.stdin.flush()
            self.assertFalse(select.select([child.stdout], [], [], 0.2)[0])
            child.stdin.close()
            child.stdin = None
            stdout, stderr = child.communicate(timeout=3)
            self.assertTrue(stdout == FIXTURE and stderr == b"" and child.returncode == 0)
        finally:
            finish(child)

    def test_read_timeout_is_bounded_and_fixed(self):
        child = subprocess.Popen([str(FILTER)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            child.wait(timeout=18)
            stdout, stderr = child.communicate(timeout=3)
            self.assertTrue(child.returncode != 0 and stdout == b"" and stderr == b"factor_pipe_failed\n")
        finally:
            finish(child)

    def test_regular_input_and_output_are_rejected(self):
        with tempfile.TemporaryFile() as regular:
            regular.write(FIXTURE)
            regular.seek(0)
            first = subprocess.run([str(FILTER)], stdin=regular, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=3)
            second = subprocess.run([str(FILTER)], input=FIXTURE, stdout=regular, stderr=subprocess.PIPE, timeout=3)
            self.assertTrue(first.returncode != 0 and first.stdout == b"" and second.returncode != 0)

    def test_arguments_are_rejected(self):
        result = subprocess.run([str(FILTER), "extra"], input=FIXTURE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=3)
        self.assertTrue(result.returncode != 0 and result.stdout == b"")

    def test_mlock_failure_is_closed_even_for_privileged_test_process(self):
        with tempfile.TemporaryDirectory(prefix="studiolo-factor-mlock-fixture-") as temporary:
            root = Path(temporary)
            shim, binary = root / "mlock.c", root / "filter"
            shim.write_text("int __wrap_mlockall(int flags){(void)flags;return -1;}\n")
            built = subprocess.run(["cc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-O2",
                str(SOURCE), str(shim), "-Wl,--wrap=mlockall", "-o", str(binary)], capture_output=True, timeout=30)
            self.assertEqual(built.returncode, 0, "虚构锁页失败替身编译失败")
            self.call(FIXTURE, binary=binary)

    def test_closed_downstream_is_rejected(self):
        read_fd, write_fd = os.pipe()
        os.close(read_fd)
        try:
            result = subprocess.run([str(FILTER)], input=FIXTURE, stdout=write_fd, stderr=subprocess.PIPE, timeout=3)
            self.assertTrue(result.returncode != 0 and result.stderr == b"factor_pipe_failed\n")
        finally:
            os.close(write_fd)

    def test_short_writes_are_completed_and_zero_write_rejected(self):
        with tempfile.TemporaryDirectory(prefix="studiolo-factor-write-fixture-") as temporary:
            root = Path(temporary)
            shim = root / "write.c"
            for zero in (False, True):
                shim.write_text('#include <unistd.h>\nssize_t __real_write(int,const void*,size_t);\n'
                    'ssize_t __wrap_write(int fd,const void*p,size_t n){if(fd==1){' +
                    ('return 0;' if zero else 'if(n>7)n=7;') + '}return __real_write(fd,p,n);}\n')
                binary = root / "filter"
                built = subprocess.run(["cc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-O2",
                    str(SOURCE), str(shim), "-Wl,--wrap=write", "-o", str(binary)], capture_output=True, timeout=30)
                self.assertEqual(built.returncode, 0, "虚构短写替身编译失败")
                self.call(FIXTURE, ok=not zero, binary=binary)


@unittest.skipUnless(sys.platform == "linux" and os.geteuid() == 0, "管线替身要求隔离 root 环境")
class PipelineTests(unittest.TestCase):
    def setUp(self):
        if not ROOT_FLOW:
            self.skipTest("须明确指定 --root-flow")
        self.temporary = tempfile.TemporaryDirectory(prefix="studiolo-portable-fixture-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.old_umask = os.umask(0o077)
        self.addCleanup(os.umask, self.old_umask)
        for name in ("portable_factor.py", "portable_factor_security.py"):
            shutil.copyfile(Path(__file__).with_name(name), self.root / name)
            (self.root / name).chmod(0o600)
        self.module = types.ModuleType("portable_fixture")
        self.module.__dict__.update(runpy.run_path(str(self.root / "portable_factor.py")))
        self.globals = self.module.pipeline.__globals__
        self.security = self.module.load_security()
        self.filter = self.root / "factor-pipe"
        shutil.copyfile(FILTER, self.filter)
        self.filter.chmod(0o700)
        self.creds, self.age = self.root / "creds", self.root / "age"
        self.credential, self.key, self.output = self.root / "credential", self.root / "host-key", self.root / "output"
        self.credential.write_bytes(b"fictional-ciphertext" * 4)
        self.key.write_bytes(b"fictional-host-key-metadata-fixture")
        self.key.chmod(0o400)
        self.script(self.creds, "import sys\nif '--version' in sys.argv: print('systemd 259 (fixture)')\nelse: sys.stdout.buffer.write(" + repr(FIXTURE) + ")\n")
        self.script(self.age, "import sys\nsys.stdin.buffer.read()\nsys.stdout.buffer.write(b'fictional-ciphertext'*5)\n")
        self.args = types.SimpleNamespace(command="export", credential=str(self.credential), recipient="age1" + "q" * 58,
            output=str(self.output), filter_bin=str(self.filter), age_bin=str(self.age), host_key=str(self.key))
        self.patch = mock.patch.dict(self.globals, SYSTEMD_CREDS=str(self.creds))
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def script(self, path, body):
        path.write_text("#!/usr/bin/python3\n" + body)
        path.chmod(0o700)

    def test_mocked_export_publishes_only_private_ciphertext(self):
        self.module.pipeline(self.args, self.security)
        self.assertEqual(self.output.stat().st_mode & 0o777, 0o600)
        self.assertFalse(list(self.root.glob(".portable-factor-partial-*")))
        self.assertNotIn(FIXTURE, self.output.read_bytes())

    def test_upstream_nonzero_and_stderr_never_publish(self):
        self.script(self.creds, "import sys\nif '--version' in sys.argv: print('systemd 259 (fixture)')\nelse:\n sys.stdout.buffer.write(" + repr(FIXTURE) + ")\n sys.stderr.buffer.write(" + repr(FIXTURE) + ")\n sys.exit(7)\n")
        with self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)
        self.assertFalse(self.output.exists())

    def test_timeout_kills_and_reaps_only_owned_children(self):
        self.script(self.age, "import time\ntime.sleep(60)\n")
        seen = []
        original = self.globals["spawn"]
        def record(*args, **kwargs):
            child = original(*args, **kwargs)
            seen.append(child)
            return child
        with mock.patch.dict(self.globals, TIMEOUT=0.2, spawn=record), self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)
        self.assertFalse(self.output.exists())
        self.assertTrue(all(child.poll() is not None for child in seen))
        for child in seen:
            with self.assertRaises(ProcessLookupError):
                os.kill(child.pid, 0)

    def test_existing_target_and_symlink_not_overwritten(self):
        self.output.write_bytes(b"sentinel")
        with self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)
        self.assertTrue(self.output.read_bytes() == b"sentinel")
        self.output.unlink()
        self.output.symlink_to(self.credential)
        with self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)

    def test_credential_modes_hardlinks_and_ancestor_links_reject(self):
        self.credential.chmod(0o644)
        with self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)
        self.credential.chmod(0o600)
        hardlink = self.root / "hardlink"
        os.link(self.credential, hardlink)
        with self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)
        hardlink.unlink()
        alias = self.root / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        self.args.credential = str(alias / "credential")
        with self.assertRaises(OSError):
            self.module.pipeline(self.args, self.security)

    def test_import_requires_private_local_key_and_pipeline_stdin(self):
        self.args.command = "import"
        self.args.host_key = None
        with self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)
        with contextlib.ExitStack() as stack:
            output = self.security.Output(str(self.output), stack)
            with self.assertRaises(ValueError):
                self.security.host_key("/var/lib/systemd/credential.secret", output, stack, True)

    def test_untrusted_tool_and_public_output_parent_reject(self):
        self.filter.chmod(0o777)
        with self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)
        self.filter.chmod(0o700)
        self.root.chmod(0o755)
        with self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)
        self.root.chmod(0o700)

    def test_cli_error_is_fixed_and_unknown_arguments_are_not_echoed(self):
        result = subprocess.run([sys.executable, "-I", "-B", "-S", str(self.root / "portable_factor.py"),
            "import", "--unknown-" + FIXTURE.decode()], capture_output=True, timeout=3)
        self.assertTrue(result.returncode != 0 and result.stdout == b"" and result.stderr == b"portable_factor_failed\n")

    def test_version_before_259_rejects_before_ciphertext_creation(self):
        self.script(self.creds, "print('systemd 258 (fixture)')\n")
        with self.assertRaises(ValueError):
            self.module.pipeline(self.args, self.security)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--filter-bin", type=Path, required=True)
    parser.add_argument("--filter-source", type=Path, default=SOURCE)
    parser.add_argument("--root-flow", action="store_true")
    args, remaining = parser.parse_known_args()
    FILTER, SOURCE, ROOT_FLOW = args.filter_bin, args.filter_source, args.root_flow
    unittest.main(argv=[sys.argv[0], *remaining])
