#!/usr/bin/env python3
"""源捕获与真实接收器的本地 SSH 替身测试，不连接服务器。"""
import fcntl
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
from replica_archive import transfer_deadline
from replica_capture import capture
from replica_client import accept_response
from replica_common import ALLOW, Invalid, digest, encode

SCRIPT = Path(__file__).absolute().with_name("sync-agent-vault.sh")


class SyncAgentVaultTests(unittest.TestCase):
    def setUp(self):
        # 夹具按默认权限建目录，而被测的捕获端与接收器按设计拒绝组／其他可写的节点。
        # Ubuntu 普通用户默认 umask 002 会让夹具目录变成 0775 而被拒；测试自己钉住前提，不依赖环境。
        self.addCleanup(os.umask, os.umask(0o022))
        self.temp = tempfile.TemporaryDirectory(dir="/private/tmp" if sys.platform == "darwin" else None)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.vault = self.root / "Owner's Vault"
        self.project = self.vault / "项目/Studiolo"
        # 同 test-replica-receiver：目录清单从 ALLOW 派生，扩白名单时无需改夹具。
        for allow in ALLOW:
            (self.vault / allow).mkdir(parents=True, exist_ok=True)
        (self.project / "note.md").write_text("private fixture text", encoding="utf-8")
        self.server = self.root / "server's copy"
        self.state = self.root / "client-state"
        bin_dir = self.root / "bin"
        bin_dir.mkdir()
        self.log = self.root / "calls.jsonl"
        stub = bin_dir / "ssh"
        stub.write_text("#!/usr/bin/env python3\nimport os, sys, json, subprocess\n"
                        "with open(os.environ['STUDIOLO_TEST_LOG'], 'a') as f:\n"
                        "    f.write(json.dumps(sys.argv[:-1]) + '\\n')\n"
                        "sys.exit(subprocess.call(['/bin/sh', '-c', sys.argv[-1]]))\n")
        stub.chmod(0o755)
        self.env = dict(os.environ, PATH=str(bin_dir) + os.pathsep + os.environ["PATH"],
                        STUDIOLO_VAULT=str(self.vault), STUDIOLO_REMOTE_REPLICA=str(self.server),
                        STUDIOLO_SYNC_STATE_DIR=str(self.state), STUDIOLO_TEST_LOG=str(self.log),
                        STUDIOLO_PYTHON=sys.executable, PYTHONDONTWRITEBYTECODE="1")

    def run_sync(self, args=(), code=0):
        result = subprocess.run(["bash", str(SCRIPT), *args], env=self.env,
                                capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, code, result.stderr)
        self.assertNotIn("private fixture text", result.stdout + result.stderr)
        return result

    def write_ssh_stub(self, source):
        stub = self.root / "bin/ssh"
        stub.write_text("#!/usr/bin/env python3\n" + textwrap.dedent(source), encoding="utf-8")
        stub.chmod(0o755)

    def test_dry_run_is_entirely_local_and_content_free(self):
        value = json.loads(self.run_sync(["--dry-run"]).stdout)
        self.assertEqual(value["status"], "dry_run")
        self.assertEqual(value["files"], 1)
        self.assertFalse(self.log.exists())
        self.assertFalse(self.server.exists())

    def test_real_run_transfers_verified_generation_in_one_ssh_call(self):
        result = json.loads(self.run_sync().stdout)
        calls = [json.loads(line) for line in self.log.read_text().splitlines()]
        self.assertEqual(len(calls), 1)
        self.assertIn("BatchMode=yes", calls[0])
        self.assertIn("StrictHostKeyChecking=yes", calls[0])
        self.assertEqual(result["status"], "ready")
        path = self.server / "releases" / result["receipt"]["generation"] / "vault/项目/Studiolo/note.md"
        self.assertEqual(path.read_text(), "private fixture text")
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def base_file(self):
        host = self.env.get("STUDIOLO_HOST", "maxsong@192.168.100.169")
        key = digest(encode([str(self.vault), host, str(self.server)]))[:32]
        return self.state / ("base-" + key + ".json")

    # SSH 替身在本机跑接收器，会继承入口给 SSH 子进程设的 ulimit -f 16（防回执填盘），
    # 所以经替身落盘的单个文件须小于 16 KiB；真实部署里接收器在远端，不受此限。
    ATTACHMENT = 12 * 1024
    # tarfile 把整包补齐到一个 10240 字节的记录；只有清单与请求的确认包正好一个记录。
    RECORD = 10240

    def record_members(self):
        """替身在内存里解析本次收到的 tar，只把成员名记下（小文件，不受 ulimit 影响）再交给接收器。"""
        members = self.root / "members.json"
        self.env["STUDIOLO_TEST_MEMBERS"] = str(members)
        self.write_ssh_stub("""
            import io, json, os, subprocess, sys, tarfile
            from pathlib import Path
            data = sys.stdin.buffer.read()
            with tarfile.open(fileobj=io.BytesIO(data)) as tar:
                names = [m.name for m in tar if m.isfile()]
            Path(os.environ['STUDIOLO_TEST_MEMBERS']).write_text(json.dumps(names, ensure_ascii=False))
            sys.exit(subprocess.run(['/bin/sh', '-c', sys.argv[-1]], input=data, timeout=8).returncode)
        """)
        return lambda: json.loads(members.read_text())

    def test_unchanged_second_run_confirms_with_manifest_only(self):
        members = self.record_members()
        (self.project / "attachment.bin").write_bytes(os.urandom(self.ATTACHMENT))
        first = json.loads(self.run_sync().stdout)
        self.assertEqual(first["mode"], "full")
        self.assertIn("vault/项目/Studiolo/attachment.bin", members())
        self.assertEqual(self.base_file().stat().st_mode & 0o777, 0o600)
        second = json.loads(self.run_sync().stdout)
        self.assertEqual(second["mode"], "confirm")
        self.assertEqual(members(), ["manifest.json", "request.json"])
        self.assertTrue(second["reused"])
        self.assertEqual(second["receipt"]["generation"], first["receipt"]["generation"])
        self.assertGreaterEqual(second["receipt"]["source_observed_at_ms"], first["receipt"]["source_observed_at_ms"])
        self.assertEqual(second["sent_bytes"], self.RECORD)

    def test_edit_sends_only_the_changed_file(self):
        members = self.record_members()
        (self.project / "attachment.bin").write_bytes(os.urandom(self.ATTACHMENT))
        self.run_sync()
        (self.project / "note.md").write_text("private fixture edit", encoding="utf-8")
        value = json.loads(self.run_sync().stdout)
        self.assertEqual(value["mode"], "delta")
        self.assertEqual(members(), ["manifest.json", "request.json", "vault/项目/Studiolo/note.md"])
        current = self.server / "releases" / value["receipt"]["generation"] / "vault/项目/Studiolo"
        self.assertEqual((current / "note.md").read_text(), "private fixture edit")
        self.assertEqual((current / "attachment.bin").read_bytes(),
                         (self.project / "attachment.bin").read_bytes())
        self.assertEqual(json.loads(self.run_sync().stdout)["mode"], "confirm")

    def test_base_mismatch_clears_base_and_next_run_sends_everything(self):
        self.run_sync()
        # 服务器被重置（或上一轮回执丢失）：本端基线已不是服务器的现代次。
        (self.server / "control/state.json").unlink()
        rejected = self.run_sync(code=1)
        self.assertEqual(json.loads(rejected.stderr), {"status": "failed", "last_error": "base_mismatch"})
        self.assertFalse(self.base_file().exists())
        value = json.loads(self.run_sync().stdout)
        self.assertEqual((value["status"], value["mode"]), ("ready", "full"))
        self.assertEqual(json.loads(self.run_sync().stdout)["mode"], "confirm")

    def test_base_from_another_allowlist_is_discarded_for_a_full_send(self):
        self.run_sync()
        stale = json.loads(self.base_file().read_bytes())
        stale["allow"] = ["_规则"]
        self.base_file().write_bytes(encode(stale))
        value = json.loads(self.run_sync().stdout)
        self.assertEqual((value["status"], value["mode"], value["reused"]), ("ready", "full", True))

    def test_unsafe_base_file_fails_closed(self):
        self.run_sync()
        self.base_file().chmod(0o644)
        self.run_sync(code=1)
        self.assertTrue(self.base_file().exists())

    def test_transfer_deadline_follows_bytes_sent(self):
        self.run_sync()
        # 替身只记下远端命令里的截止与实际收到的字节数，不落盘（避开 ulimit），回失败让基线保持不变。
        captured = self.root / "deadline.json"
        self.env["STUDIOLO_TEST_DEADLINE"] = str(captured)
        self.write_ssh_stub("""
            import json, os, shlex, sys
            from pathlib import Path
            words = shlex.split(sys.argv[-1])
            size = len(sys.stdin.buffer.read())
            Path(os.environ['STUDIOLO_TEST_DEADLINE']).write_text(json.dumps(
                [int(words[words.index('--deadline') + 1]), size]))
            sys.exit(1)
        """)
        (self.project / "attachment.bin").write_bytes(os.urandom(3 * 1024 * 1024))
        self.run_sync(code=1)
        deadline, size = json.loads(captured.read_text())
        self.assertGreater(size, 3 * 1024 * 1024)
        self.assertEqual(deadline, transfer_deadline(size))
        self.assertEqual(deadline, 124)
        (self.project / "attachment.bin").unlink()
        self.run_sync(code=1)
        deadline, size = json.loads(captured.read_text())
        self.assertEqual(size, self.RECORD)
        self.assertEqual(deadline, 121)

    def test_ssh_is_started_directly_by_bash(self):
        parent_log = self.root / "ssh-parent"
        self.env["STUDIOLO_TEST_PARENT"] = str(parent_log)
        self.write_ssh_stub("""
            import os, subprocess, sys
            from pathlib import Path
            parent = subprocess.check_output(
                ['ps', '-p', str(os.getppid()), '-o', 'comm='], text=True, timeout=2)
            Path(os.environ['STUDIOLO_TEST_PARENT']).write_text(parent.strip())
            sys.exit(subprocess.run(['/bin/sh', '-c', sys.argv[-1]], timeout=8).returncode)
        """)
        self.assertEqual(json.loads(self.run_sync().stdout)["status"], "ready")
        self.assertEqual(Path(parent_log.read_text().strip()).name, "bash")

    def test_source_lock_is_held_while_ssh_waits_for_receipt(self):
        started = self.root / "ssh-started"
        release = self.root / "ssh-release"
        self.env.update(STUDIOLO_TEST_STARTED=str(started), STUDIOLO_TEST_RELEASE=str(release))
        self.write_ssh_stub("""
            import os, subprocess, sys, time
            from pathlib import Path
            Path(os.environ['STUDIOLO_TEST_STARTED']).touch()
            release = Path(os.environ['STUDIOLO_TEST_RELEASE'])
            deadline = time.monotonic() + 10
            while not release.exists():
                if time.monotonic() >= deadline:
                    sys.exit(124)
                time.sleep(0.01)
            sys.exit(subprocess.run(['/bin/sh', '-c', sys.argv[-1]], timeout=8).returncode)
        """)
        first = subprocess.Popen(["bash", str(SCRIPT)], env=self.env, stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE, text=True, start_new_session=True)
        deadline = time.monotonic() + 10
        try:
            while not started.exists() and first.poll() is None and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(started.exists(), "SSH 未在 10 秒内进入文件屏障")
            self.assertIsNone(first.poll(), "第一批同步在收到回执之前退出")
            rejected = self.run_sync(["--dry-run"], code=1)
            self.assertEqual(json.loads(rejected.stderr)["status"], "failed")
            self.assertIsNone(first.poll(), "第二批检查结束前第一批已退出，未证明锁重叠")
            release.touch()
            stdout, stderr = first.communicate(timeout=max(0.1, deadline - time.monotonic()))
            self.assertEqual(first.returncode, 0, stderr)
            self.assertNotIn("private fixture text", stdout + stderr)
            self.assertEqual(json.loads(stdout)["status"], "ready")
        finally:
            release.touch()
            # 子会话只含此用例的同步进程；失败也必须回收 SSH 与接收器后代。
            try:
                os.killpg(first.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                first.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(first.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                first.communicate(timeout=2)
        self.assertEqual(json.loads(self.run_sync(["--dry-run"]).stdout)["status"], "dry_run")

    def test_ssh_failure_does_not_reflect_either_output_stream(self):
        self.write_ssh_stub("""
            import sys
            print('private fixture text')
            print('private fixture text', file=sys.stderr)
            sys.exit(17)
        """)
        result = self.run_sync(code=1)
        self.assertEqual(result.stdout, "")
        self.assertEqual(json.loads(result.stderr)["status"], "failed")

    @unittest.skipUnless(Path("/proc/self/fd/0").exists(), "需要 Linux /proc 文件描述符路径")
    def test_transfer_timeout_reaps_ssh_and_removes_private_archive(self):
        self.write_ssh_stub("""
            import json, os, time
            from pathlib import Path
            Path(os.environ['STUDIOLO_TEST_LOG']).write_text(json.dumps({
                'pid': os.getpid(), 'archive': os.readlink('/proc/self/fd/0')}))
            time.sleep(10)
        """)
        sleep = self.root / "bin/sleep"
        sleep.write_text("#!/usr/bin/env python3\n" + textwrap.dedent("""
            import os, sys, time
            from pathlib import Path
            # 看门狗有两段（120 秒捕获段、按字节数给的传输段），都是 ≥120 的整数秒；其余 sleep 照常。
            if not (len(sys.argv) == 2 and sys.argv[1].isdigit() and int(sys.argv[1]) >= 120):
                os.execv('/bin/sleep', ['sleep', *sys.argv[1:]])
            # 确定 SSH 已开始再压缩 watchdog 时间，避免只测到准备阶段超时。
            deadline = time.monotonic() + 8
            while not Path(os.environ['STUDIOLO_TEST_LOG']).exists():
                if time.monotonic() >= deadline:
                    sys.exit(124)
                time.sleep(0.01)
            time.sleep(0.8)
        """), encoding="utf-8")
        sleep.chmod(0o755)
        process = subprocess.Popen(["bash", str(SCRIPT)], env=self.env, stdout=subprocess.PIPE,
                                   stderr=subprocess.PIPE, text=True, start_new_session=True)
        try:
            stdout, stderr = process.communicate(timeout=5)
            self.assertEqual(process.returncode, 1, stderr)
            self.assertEqual(stdout, "")
            self.assertNotIn("private fixture text", stderr)
            self.assertEqual(json.loads(stderr), {"status": "failed", "last_error": "transfer_timeout"})
            child = json.loads(self.log.read_text())
            # 此处必须在用例自身 finally 清理之前检查，证明清理来自同步入口。
            self.assertFalse(Path("/proc", str(child["pid"])).exists(), "超时后 SSH 进程仍存在")
            self.assertFalse(Path(child["archive"]).parent.exists(), "超时后私有归档目录仍存在")
        finally:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.communicate(timeout=2)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.communicate(timeout=2)

    def test_malformed_and_oversized_ssh_receipts_cannot_report_ready(self):
        replies = {
            "malformed": "import sys\nsys.stdout.write('{\"status\":\"ready\",private fixture text')\n",
            # 有效回执后加空白仍是合法 JSON；必须按完整字节数拒绝，不能只截读前缀。
            "oversized": """
                import subprocess, sys
                result = subprocess.run(['/bin/sh', '-c', sys.argv[-1]],
                                        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=8)
                sys.stdout.buffer.write(result.stdout + b' ' * 8193)
                sys.exit(result.returncode)
            """,
        }
        for name, source in replies.items():
            with self.subTest(reply=name):
                self.write_ssh_stub(source)
                result = self.run_sync(code=1)
                self.assertEqual(result.stdout, "")
                self.assertEqual(json.loads(result.stderr)["status"], "failed")

    def test_shell_metacharacters_in_remote_path_are_quoted(self):
        marker = self.root / "injected"
        self.server = self.root / "server;$(touch injected) 'copy'"
        self.env["STUDIOLO_REMOTE_REPLICA"] = str(self.server)
        self.run_sync()
        self.assertTrue((self.server / "control/state.json").is_file())
        self.assertFalse(marker.exists())
        self.assertFalse(SCRIPT.parent.parent.joinpath("injected").exists())

    def test_extra_argument_rejected_before_remote_access(self):
        self.run_sync(["--dry-run", "unexpected"], 2)
        self.assertFalse(self.log.exists())

    def test_host_option_injection_is_rejected(self):
        self.env["STUDIOLO_HOST"] = "-oProxyCommand=touch"
        self.run_sync(code=1)
        self.assertFalse(self.log.exists())

    def test_excluded_hidden_subtree_is_not_traversed(self):
        hidden = self.project / ".ignored"
        hidden.mkdir()
        (hidden / "link").symlink_to(self.root)
        (self.vault / "outside.md").write_text("outside")
        value = json.loads(self.run_sync(["--dry-run"]).stdout)
        self.assertEqual(value["files"], 1)

    def test_visible_symlink_rejected_before_remote_access(self):
        (self.project / "link").symlink_to(self.root)
        self.run_sync(["--dry-run"], 1)
        self.assertFalse(self.log.exists())

    def test_visible_hard_link_rejected(self):
        os.link(self.project / "note.md", self.project / "second.md")
        self.run_sync(["--dry-run"], 1)

    def test_visible_fifo_rejected_without_blocking(self):
        os.mkfifo(self.project / "pipe")
        self.run_sync(["--dry-run"], 1)

    def test_middle_parent_symlink_rejected(self):
        alias = self.root / "alias"
        alias.symlink_to(self.vault.parent, target_is_directory=True)
        self.env["STUDIOLO_VAULT"] = str(alias / self.vault.name)
        self.run_sync(["--dry-run"], 1)

    def test_allow_parent_symlink_rejected(self):
        (self.vault / "项目").rename(self.vault / "real-project")
        (self.vault / "项目").symlink_to(self.vault / "real-project")
        self.run_sync(["--dry-run"], 1)

    def test_source_group_write_rejected(self):
        (self.project / "note.md").chmod(0o660)
        self.run_sync(["--dry-run"], 1)

    def test_source_change_during_capture_rejects_entire_batch(self):
        def mutate():
            (self.project / "note.md").write_text("changed")
        with self.assertRaises(Invalid):
            capture(str(self.vault), between_scans=mutate)

    def test_source_delete_during_capture_rejected(self):
        with self.assertRaises(Invalid):
            capture(str(self.vault), between_scans=lambda: (self.project / "note.md").unlink())

    def test_source_added_empty_directory_during_capture_rejected(self):
        with self.assertRaises(Invalid):
            capture(str(self.vault), between_scans=lambda: (self.project / "empty").mkdir())

    def test_source_same_bytes_replaced_inode_rejected(self):
        def replace():
            note = self.project / "note.md"
            raw = note.read_bytes()
            note.unlink()
            note.write_bytes(raw)
        with self.assertRaises(Invalid):
            capture(str(self.vault), between_scans=replace)

    def test_source_lock_prevents_concurrent_capture(self):
        self.state.mkdir(mode=0o700)
        host = self.env.get("STUDIOLO_HOST", "maxsong@192.168.100.169")
        key = digest(encode([str(self.vault), host, str(self.server)]))[:32]
        with open(self.state / ("sync-" + key + ".lock"), "wb") as locked:
            os.fchmod(locked.fileno(), 0o600)
            fcntl.flock(locked, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.run_sync(["--dry-run"], 1)
        self.assertFalse(self.log.exists())

    def test_all_names_enumeration_is_bounded_including_hidden(self):
        for n in range(6):
            (self.project / (".hidden" + str(n))).touch()
        with patch("replica_capture.MAX_ENTRIES", 5), self.assertRaises(Invalid):
            capture(str(self.vault))

    def test_total_markdown_budget_and_utf8_are_checked_at_source(self):
        (self.project / "note.md").write_bytes(b"one!")
        (self.project / "second.md").write_bytes(b"two!")
        with patch("replica_capture.MAX_NOTES_TOTAL", 6), self.assertRaises(Invalid):
            capture(str(self.vault))
        (self.project / "note.md").write_bytes(b"\xff")
        self.run_sync(["--dry-run"], 1)
        self.assertFalse(self.log.exists())

    def test_remote_ack_must_match_this_capture_without_extra_fields(self):
        good = {"status": "ready", "reused": False, "receipt": {
            "generation": "a" * 32, "source_digest": "b" * 64,
            "source_observed_at_ms": 100, "received_at_ms": 101}}
        self.assertEqual(accept_response(good, "b" * 64, 100), good)
        for change in (
            lambda v: v.update(unexpected="private fixture text"),
            lambda v: v.update(reused="yes"),
            lambda v: v["receipt"].update(unexpected="private fixture text"),
            lambda v: v["receipt"].update(source_digest="c" * 64),
            lambda v: v["receipt"].update(source_observed_at_ms=99),
            lambda v: v["receipt"].update(generation="../../escape"),
            lambda v: v["receipt"].update(received_at_ms=True),
            lambda v: v["receipt"].update(received_at_ms=99),
        ):
            value = json.loads(json.dumps(good))
            change(value)
            with self.assertRaises(Invalid):
                accept_response(value, "b" * 64, 100)


if __name__ == "__main__":
    unittest.main()
