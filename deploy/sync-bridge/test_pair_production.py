"""仅使用临时文件和假进程验证配对入口，不访问生产目录或凭据。"""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("pair_production", Path(__file__).with_name("pair-production.py"))
pair = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(pair)


def configured(**changes):
    value = {"vaultId": "a" * 32, "vaultName": pair.VAULT, "vaultPath": str(pair.MIRROR),
             "syncMode": "mirror-remote", "conflictStrategy": "conflict", "deviceName": pair.DEVICE,
             "configDir": ".obsidian", "configs": [], "fileTypes": list(pair.FILE_TYPES),
             "excludedFolders": list(pair.EXCLUDED)}
    value.update(changes)
    return json.dumps(value).encode()


class PairProductionTests(unittest.TestCase):
    def invoke(self, process=None, preflight=None, args=()):
        # 生产入口的环境清理不影响测试运行器；从不打开真实 /dev/tty。
        output, errors = io.StringIO(), io.StringIO()
        fd = os.open(os.devnull, os.O_RDONLY)
        with contextlib.ExitStack() as stack:
            stack.enter_context(patch.dict(os.environ, {"PASSWORD": "sentinel-secret"}))
            stack.enter_context(patch.object(pair.os, "umask"))
            stack.enter_context(patch.object(pair, "terminal", return_value=fd))
            stack.enter_context(patch.object(pair, "preflight", side_effect=preflight))
            run = stack.enter_context(patch.object(pair.subprocess, "run", side_effect=process))
            stack.enter_context(contextlib.redirect_stdout(output))
            stack.enter_context(contextlib.redirect_stderr(errors))
            result = pair.main(list(args))
        # 参数拒绝发生在取得终端前，入口不会拥有这个测试描述符。
        if args:
            os.close(fd)
        return result, output.getvalue(), errors.getvalue(), run

    def test_only_fixed_commands_with_tty_and_clean_environment(self):
        def process(argv, **options):
            index = argv.index(str(pair.CLI))
            verb = argv[index + 1]
            self.assertNotIn("sentinel-secret", repr((argv, options)))
            self.assertEqual(options["env"], pair.ENV)
            self.assertEqual(dict(os.environ), pair.ENV)
            self.assertNotIn("--password", argv)
            self.assertNotIn("shell", options)
            if verb != "sync-config":
                self.assertNotIn("stdout", options)
                self.assertNotIn("stderr", options)
                self.assertNotIn("stdin", options)
                self.assertNotIn("--json", argv)
            else:
                self.assertEqual(options["stderr"], subprocess.DEVNULL)
                self.assertEqual(options["stdout"], subprocess.PIPE)
            return SimpleNamespace(returncode=0, stdout=configured())

        status, output, errors, run = self.invoke(process)
        self.assertEqual(status, 0)
        self.assertEqual(errors, "")
        self.assertEqual(output, "pairing_configured=true mode=mirror-remote excluded_folders=5 file_types=5 configs=0\n")
        self.assertEqual(pair.ENV, {"PATH": "/usr/bin:/bin", "LANG": "C.UTF-8",
                                  "HOME": "/var/lib/studiolo-sync",
                                  "XDG_CONFIG_HOME": "/var/lib/studiolo-sync/config"})
        suffixes = [call.args[0][call.args[0].index(str(pair.CLI)) + 1:] for call in run.call_args_list]
        self.assertEqual(suffixes, [
            ["login"],
            ["sync-setup", "--vault", "Max's Secret Place", "--path", str(pair.MIRROR),
             "--device-name", "studiolo-169-bridge"],
            ["sync-config", "--path", str(pair.MIRROR), "--mode", "mirror-remote",
             "--conflict-strategy", "conflict", "--excluded-folders", "School,日记,大学申请,周报+日报,Ninth English",
             "--file-types", "image,audio,video,pdf,unsupported", "--configs", "",
             "--device-name", "studiolo-169-bridge", "--json"]])
        for call in run.call_args_list:
            self.assertEqual(call.args[0][:6], ["/usr/sbin/runuser", "-u", "studiolo-sync",
                                               "--", "/usr/bin/env", "-i"])
        verbs = [call.args[0][call.args[0].index(str(pair.CLI)) + 1] for call in run.call_args_list]
        self.assertEqual(verbs, ["login", "sync-setup", "sync-config"])
        self.assertNotIn("sync", verbs)
        with self.assertRaises(pair.Refused):
            pair.command("sync")

    def test_parameters_or_failed_preflight_never_start_process(self):
        for args, refusal in ((("--password", "sentinel-secret"), None),
                              ((), pair.Refused("mirror-not-empty")), ((), pair.Refused("p3"))):
            with self.subTest(args=args, refusal=refusal):
                status, output, errors, run = self.invoke(preflight=refusal, args=args)
                self.assertEqual(status, 1)
                self.assertEqual(output, "")
                self.assertNotIn("sentinel-secret", errors)
                run.assert_not_called()

    def test_operator_and_each_non_tty_are_rejected(self):
        for platform, uid, bad in (("darwin", 0, None), ("linux", 501, None),
                                   ("linux", 0, 0), ("linux", 0, 1), ("linux", 0, 2)):
            streams = [SimpleNamespace(isatty=lambda: True) for _ in range(3)]
            if bad is not None:
                streams[bad] = SimpleNamespace(isatty=lambda: False)
            with patch.object(pair.sys, "platform", platform), patch.object(pair.os, "getuid", return_value=uid), \
                    patch.object(pair.os, "geteuid", return_value=uid), \
                    patch.object(pair.sys, "stdin", streams[0]), patch.object(pair.sys, "stdout", streams[1]), \
                    patch.object(pair.sys, "stderr", streams[2]), patch.object(pair.os, "open") as opened:
                with self.assertRaises(pair.Refused):
                    pair.terminal()
                opened.assert_not_called()

    def test_missing_controlling_terminal_fails_before_preflight(self):
        with patch.object(pair, "terminal", side_effect=OSError("sentinel-secret")), \
                patch.object(pair, "preflight") as preflight, patch.dict(os.environ), \
                patch.object(pair.os, "umask"), contextlib.redirect_stderr(io.StringIO()) as errors:
            self.assertEqual(pair.main([]), 1)
            preflight.assert_not_called()
            self.assertEqual(errors.getvalue(), "pairing_failed stage=tty exit_code=1\n")

    def test_runtime_metadata_rejects_links_owner_and_writes(self):
        good = SimpleNamespace(st_mode=stat.S_IFREG | 0o555, st_uid=0)
        for mode, uid in ((stat.S_IFREG | 0o575, 0), (stat.S_IFREG | 0o555, 999), (stat.S_IFDIR | 0o555, 0)):
            with patch.object(pair, "chain", return_value=[SimpleNamespace(st_mode=mode, st_uid=uid)]):
                with self.assertRaises(pair.Refused):
                    pair.trusted_file(pair.NODE)
        with patch.object(pair, "chain", return_value=[good]):
            self.assertIs(pair.trusted_file(pair.NODE), good)
        with patch.object(pair, "chain", return_value=[SimpleNamespace(st_mode=0o777, st_uid=0), good]):
            with self.assertRaises(pair.Refused):
                pair.trusted_file(pair.NODE)
        with tempfile.TemporaryDirectory() as directory:
            leaf = Path(directory) / "link"
            leaf.symlink_to("missing")
            with self.assertRaises(pair.Refused):
                pair.chain(leaf)

    def test_wrong_identity_supplementary_groups_and_directory_mode_are_rejected(self):
        for uid, groups in ((998, [983]), (999, [983, 10])):
            with patch.object(pair.pwd, "getpwnam", return_value=SimpleNamespace(pw_uid=uid, pw_gid=983)), \
                    patch.object(pair.os, "getgrouplist", return_value=groups):
                with self.assertRaises(pair.Refused) as failure:
                    pair.preflight()
                self.assertEqual(failure.exception.stage, "identity")
        for uid, gid, mode in ((998, 983, 0o700), (999, 984, 0o700), (999, 983, 0o750)):
            info = SimpleNamespace(st_uid=uid, st_gid=gid, st_mode=stat.S_IFDIR | mode)
            with patch.object(pair, "chain", return_value=[info]), self.assertRaises(pair.Refused):
                pair.private_directory(pair.MIRROR)

    def test_receipt_is_existing_private_accepted_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "accepted.json"
            with patch.object(pair, "RECEIPT", path), patch.object(pair, "trusted_file", side_effect=lambda _: path.lstat()):
                with self.assertRaises(FileNotFoundError):
                    pair.accepted()
                self.assertFalse(path.exists())
                for raw, mode in ((b'{"status":"rejected"}', 0o600),
                                  (b'{"status":"accepted"}', 0o644),
                                  (b'{"status":"accepted","status":"accepted"}', 0o600)):
                    path.write_bytes(raw)
                    path.chmod(mode)
                    with self.assertRaises(pair.Refused):
                        pair.accepted()
                path.write_bytes(b'{"status":"accepted","other":"unused"}')
                path.chmod(0o600)
                pair.accepted()

    def test_mount_must_be_unique_loop_root_on_expected_device(self):
        good = f"42 1 7:0 / {pair.MOUNT} rw - ext4 /dev/loop0 rw\n"
        pair.loop_device(good, "7:0")
        for value in ("", good + good, good.replace("7:0", "8:0"),
                      good.replace("/dev/loop0", "/dev/sda"), good.replace(" / ", " /subtree ")):
            with self.assertRaises(pair.Refused):
                pair.loop_device(value, "7:0")

    def test_preflight_rejects_nonempty_mirror_and_wrong_backing(self):
        with tempfile.TemporaryDirectory() as directory:
            mirror = Path(directory)
            info = mirror.stat()
            runtime = SimpleNamespace(st_mode=0o555)
            with contextlib.ExitStack() as stack:
                stack.enter_context(patch.object(pair.pwd, "getpwnam", return_value=SimpleNamespace(pw_uid=999, pw_gid=983)))
                stack.enter_context(patch.object(pair.os, "getgrouplist", return_value=[983]))
                stack.enter_context(patch.object(pair, "trusted_file", return_value=runtime))
                stack.enter_context(patch.object(pair, "accepted"))
                stack.enter_context(patch.object(pair, "chain", return_value=[info]))
                stack.enter_context(patch.object(pair, "private_directory", return_value=info))
                stack.enter_context(patch.object(pair, "loop_device"))
                stack.enter_context(patch.object(pair, "MIRROR", mirror))
                read = stack.enter_context(patch.object(Path, "read_text", side_effect=["mounts", "wrong"]))
                with self.assertRaises(pair.Refused) as failure:
                    pair.preflight()
                self.assertEqual(failure.exception.stage, "mount")
                read.side_effect = ["mounts", pair.IMAGE]
                (mirror / "existing").write_text("fixture")
                with self.assertRaises(pair.Refused) as failure:
                    pair.preflight()
                self.assertEqual(failure.exception.stage, "mirror-not-empty")

    def test_json_closed_fields_identity_and_approved_settings(self):
        pair.verify(configured())
        changes = ({"vaultId": "0" * 32}, {"vaultId": "A" * 32}, {"vaultId": []},
                   {"syncMode": "bidirectional"}, {"vaultName": "different"}, {"vaultPath": "/other"},
                   {"conflictStrategy": "merge"}, {"deviceName": "other"}, {"configDir": ".other"},
                   {"configs": ["appearance"]}, {"fileTypes": ["image"] * 5},
                   {"excludedFolders": []}, {"password": "sentinel-secret"})
        for change in changes:
            with self.subTest(change=change), self.assertRaises(pair.Refused):
                pair.verify(configured(**change))
        for raw in (b'[]', b'{"vaultId":"a","vaultId":"b"}', b'x' * (pair.LIMIT + 1), b'\xff'):
            with self.assertRaises(pair.Refused):
                pair.verify(raw)

    def test_child_failures_and_invalid_json_never_leak_body(self):
        for failed, expected_calls in (("login", 1), ("sync-setup", 2)):
            def early_failure(argv, **options):
                return SimpleNamespace(returncode=23 if failed in argv else 0, stdout=b"sentinel-secret")
            status, output, errors, run = self.invoke(early_failure)
            self.assertEqual(status, 1)
            self.assertEqual(output, "")
            self.assertEqual(run.call_count, expected_calls)
            self.assertIn("exit_code=23", errors)
            self.assertNotIn("sentinel-secret", errors)
        for code, raw in ((17, b"sentinel-secret"), (0, b"sentinel-secret")):
            def process(argv, **options):
                return SimpleNamespace(returncode=code if "sync-config" in argv else 0, stdout=raw)
            status, output, errors, run = self.invoke(process)
            self.assertEqual(status, 1)
            self.assertEqual(output, "")
            self.assertNotIn("sentinel-secret", errors)
            self.assertEqual(run.call_count, 3)
        for exception, code in ((OSError("sentinel-secret"), 127),
                                (subprocess.TimeoutExpired("sentinel-secret", 60), 124)):
            with patch.object(pair.subprocess, "run", side_effect=exception):
                with self.assertRaises(pair.Refused) as failure:
                    pair.run("configure")
                self.assertEqual((failure.exception.stage, failure.exception.code), ("configure", code))


if __name__ == "__main__":
    unittest.main()
