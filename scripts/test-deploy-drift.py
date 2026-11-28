#!/usr/bin/env python3
"""deploy-drift.py 回归：临时 git 仓库、虚构发行记录；不联网、不读真 vault、不碰 169。"""
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
SCRIPTS = Path(__file__).resolve().parent
SCRIPT = SCRIPTS / "deploy-drift.py"
WORKSPACE = SCRIPTS.parent
spec = importlib.util.spec_from_file_location("deploy_drift", SCRIPT)
drift = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drift)

GIT = shutil.which("git")
# 合成的仓库里各发行的路径：依赖闭包要从 Cargo.toml 真读出来，不许按目录名猜。
CARGO = {
    "core": [],
    "audit-ledger": ["core"],
    "observability": ["core"],
    "supply": ["core"],
    "auth": ["core"],
    "runner-adapter": ["core"],
    "local-server": ["auth", "audit-ledger", "observability", "runner-adapter"],
    "sync-launcher": ["core"],
    "agent-server": ["supply", "audit-ledger"],
    "backup-worker": ["core"],
    "studiolo": ["core"],
}
PARTS = ("人侧程序", "人侧网页", "人侧随包工具", "agent", "Mac 副本包", "告警包", "Worker")


def manifest(name, depends):
    lines = ['[package]', f'name = "studiolo-{name}"', '', '[dependencies]']
    lines += [f'studiolo-{other}.workspace = true' for other in depends]
    # 开发依赖不进产物：agent-server 的测试用到 local-server，也不能让它算进 agent 的闭包。
    lines += ['', '[dev-dependencies]', 'studiolo-local-server.workspace = true', '']
    return "\n".join(lines)


class Sandbox:
    """一个有 main 的临时仓库、一份 169 记录、一个假 06-BUG、一个只有 git 和假网络程序的 PATH。"""

    def __init__(self, directory):
        self.root = Path(directory)
        self.repo = self.root / "repo"
        self.repo.mkdir()
        self.bugs = self.root / "bugs"
        self.bugs.mkdir()
        self.home = self.root / "home"
        self.home.mkdir()
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.called = self.root / "called"
        (self.bin / "git").symlink_to(GIT)
        for name in ("ssh", "scp", "rsync", "curl", "sudo", "systemctl"):
            fake = self.bin / name
            fake.write_text(f"#!/bin/sh\necho {name} >> '{self.called}'\nexit 1\n")
            fake.chmod(0o755)
        self.git("init", "-q", "-b", "main")
        files = {"Cargo.toml": "[workspace]\n", "Cargo.lock": "# 1\n", "rust-toolchain.toml": "# 1\n",
                 "web/src/App.tsx": "1\n", "web/index.html": "1\n", "docs/a.md": "1\n",
                 "scripts/sync-agent-vault.sh": "1\n", "scripts/replica_common.py": "1\n",
                 "scripts/check-agent.py": "1\n", "scripts/check-agent-replica.py": "1\n",
                 "deploy/alerts/run.py": "1\n", "deploy/alerts/worker/src/worker.mjs": "1\n",
                 "deploy/runner/launch.py": "1\n", "deploy/runner/rg_probe.py": "1\n",
                 "deploy/agent/studiolo-agent-system.service.in": "1\n",
                 "deploy/backup-worker/systemd/x.service": "1\n"}
        for name, depends in CARGO.items():
            files[f"crates/{name}/Cargo.toml"] = manifest(name, depends)
            files[f"crates/{name}/src/lib.rs"] = "1\n"
        files["crates/local-server/src/alert_main.rs"] = "1\n"
        self.base = self.commit("初始", files)

    def git(self, *args):
        environment = {"PATH": os.environ["PATH"], "HOME": str(self.home), "LC_ALL": "C.UTF-8",
                       "GIT_CONFIG_NOSYSTEM": "1", "GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@example.invalid",
                       "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@example.invalid"}
        return subprocess.run([GIT, "-C", str(self.repo), *args], check=True, capture_output=True, text=True,
                              env=environment, timeout=60).stdout.strip()

    def commit(self, message, files):
        for name, body in files.items():
            path = self.repo / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(body)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", message)
        return self.git("rev-parse", "HEAD")

    def touch(self, message, path):
        """在这个路径上追加一行并提交。"""
        target = self.repo / path
        body = target.read_text() if target.exists() else ""
        return self.commit(message, {path: body + message + "\n"})

    def record(self, **overrides):
        """169 的记录；每项缺省都指向初始提交。"""
        value = {
            "schema": 1,
            "collected_at": "2026-11-28T10:00:00Z",
            "human": {"release": "m5m6-board-20260920-r27", "build": "r27",
                      "human_program_source_commit": self.base, "web_source_commit": self.base,
                      "sync_program_source_commit": self.base, "bootstrap_source_commit": self.base},
            "agent": {"release": "t215-116f5d7-20261120", "commit": self.base},
            "alert": {"commit": self.base},
        }
        value.update(overrides)
        return value

    def human(self, **overrides):
        return {**self.record()["human"], **overrides}

    def report(self, record, *extra, mac=True, worker=True, main="main"):
        path = self.root / "record.json"
        if isinstance(record, (bytes, str)):
            path.write_bytes(record.encode() if isinstance(record, str) else record)
        elif record is not None:
            path.write_text(json.dumps(record))
        arguments = ["report", "--record", str(path), "--repo", str(self.repo), "--main", main,
                     "--bugs", str(self.bugs)]
        if mac:
            arguments += ["--mac", mac if isinstance(mac, str) else self.base]
        if worker:
            arguments += ["--worker", worker if isinstance(worker, str) else self.base]
        return self.run(*arguments, *extra)

    def run(self, *arguments, stdin=None):
        result = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), *arguments], capture_output=True, text=True,
                                input=stdin, timeout=120,
                                env={"PATH": str(self.bin), "HOME": str(self.home), "LC_ALL": "C.UTF-8"})
        return result.returncode, result.stdout, result.stderr

    def bug(self, number, status, body=""):
        (self.bugs / f"BUG-{number:03d}-测试.md").write_text(
            f"---\nid: BUG-{number:03d}\n状态: {status}\n严重级: P1\n---\n\n# BUG-{number:03d} 标题{number}\n\n{body}\n")


def section(text, title):
    """取一节：从标题行到下一个二级标题。"""
    found = re.search(r"^## " + re.escape(title) + r".*?(?=^## |\Z)", text, re.M | re.S)
    if not found:
        raise AssertionError(f"没有这一节：{title}\n{text}")
    return found.group(0)


class ReportTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.box = Sandbox(directory.name)

    def test_everything_is_live(self):
        code, out, err = self.box.report(self.box.record())
        self.assertEqual((code, err), (0, ""), out)
        for part in PARTS:
            self.assertIn("没有漂移", section(out, part), part)
        self.assertIn("结论：五个发行都没有漂移，没有该验收的 BUG。", out)
        self.assertFalse(self.box.called.exists())

    def test_changes_that_reach_no_release_are_not_drift(self):
        for path in ("docs/a.md", "crates/local-server/tests/x.rs", "crates/auth/src/session_tests.rs",
                     "crates/agent-server/src/options/tests.rs", "web/src/App.test.tsx", "scripts/test-check-agent.py",
                     "deploy/alerts/test_monitor.py", "deploy/alerts/README.md", "deploy/alerts/worker/test/a.mjs",
                     "crates/studiolo/src/lib.rs", "bottega/Bottega/App.swift", "AGENTS.md",
                     "deploy/blackboard/assembly.py"):
            self.box.touch("只动 " + path, path)
        code, out, _ = self.box.report(self.box.record())
        self.assertEqual(code, 0, out)

    def test_each_release_lists_exactly_the_commits_it_lacks(self):
        box = self.box
        auth = box.touch("fix: 认证", "crates/auth/src/lib.rs")
        web = box.touch("feat: 网页", "web/src/App.tsx")
        launcher = box.touch("fix: 启动器", "crates/sync-launcher/src/main.rs")
        supply = box.touch("fix: 供给", "crates/supply/src/lib.rs")
        unit = box.touch("fix: agent 单元", "deploy/agent/studiolo-agent-system.service.in")
        mac = box.touch("fix: 副本", "scripts/replica_common.py")
        alert = box.touch("fix: 告警", "deploy/alerts/run.py")
        worker = box.touch("fix: 心跳", "deploy/alerts/worker/src/worker.mjs")
        embedded = box.touch("fix: 内嵌探针", "deploy/runner/rg_probe.py")
        ledger = box.touch("fix: 账本", "crates/audit-ledger/src/lib.rs")
        code, out, _ = box.report(box.record())
        self.assertEqual(code, 1, out)
        expected = {"人侧程序": {auth, embedded, ledger}, "人侧网页": {web}, "人侧随包工具": {launcher},
                    "agent": {supply, unit, ledger}, "Mac 副本包": {mac}, "告警包": {alert, ledger}, "Worker": {worker}}
        everything = {auth, web, launcher, supply, unit, mac, alert, worker, embedded, ledger}
        for part, commits in expected.items():
            text = section(out, part)
            listed = {commit for commit in everything if re.search("^- " + commit[:7], text, re.M)}
            self.assertEqual(listed, commits, f"{part}\n{text}")
            self.assertIn(f"{len(commits)} 个提交已合 main、不在线上", text)
            self.assertNotIn("没有漂移", text)
        self.assertIn("结论：有漂移", out)

    def test_a_release_that_caught_up_only_lists_what_came_after(self):
        box = self.box
        first = box.touch("fix: 一", "crates/auth/src/lib.rs")
        second = box.touch("fix: 二", "crates/auth/src/lib.rs")
        code, out, _ = box.report(box.record(human=box.human(human_program_source_commit=first)))
        text = section(out, "人侧程序")
        self.assertEqual(code, 1)
        self.assertNotIn(first[:7] + " ", text.split("\n", 1)[1])
        self.assertIn("- " + second[:7], text)

    def test_merges_are_followed_into_the_commits_they_bring(self):
        box = self.box
        box.git("checkout", "-q", "-b", "side")
        side = box.touch("fix: 分支上的", "crates/auth/src/lib.rs")
        box.git("checkout", "-q", "main")
        box.touch("docs", "docs/a.md")
        box.git("merge", "-q", "--no-ff", "-m", "merge", "side")
        _, out, _ = box.report(box.record())
        self.assertIn("- " + side[:7], section(out, "人侧程序"))

    def test_a_change_made_only_inside_a_merge_is_still_reported(self):
        box = self.box
        box.git("checkout", "-q", "-b", "side")
        box.touch("docs", "docs/b.md")
        box.git("checkout", "-q", "main")
        box.touch("docs", "docs/a.md")
        box.git("merge", "-q", "--no-ff", "--no-commit", "side")
        box.commit("merge", {"crates/auth/src/lib.rs": "合并里夹带的改动\n"})
        code, out, _ = box.report(box.record())
        text = section(out, "人侧程序")
        self.assertEqual(code, 1)
        self.assertNotIn("没有漂移", text)
        self.assertIn("合并提交里带的改动", text)
        self.assertIn("crates/auth/src/lib.rs", text)

    def test_a_live_commit_outside_main_is_listed_not_compared(self):
        box = self.box
        box.git("checkout", "-q", "-b", "side")
        lost = box.touch("分支上的发行", "crates/auth/src/lib.rs")
        box.git("checkout", "-q", "main")
        code, out, _ = box.report(box.record(agent={"release": "t999-x", "commit": lost}))
        self.assertEqual(code, 1)
        text = section(out, "agent")
        self.assertIn("线上源提交不在 main", text)
        self.assertNotIn("没有漂移", text)
        for part in set(PARTS) - {"agent"}:
            self.assertIn("没有漂移", section(out, part), part)

    def test_a_release_nobody_recorded_is_unknown_not_clean(self):
        box = self.box
        code, out, _ = box.report(box.record(), worker=False)
        self.assertEqual(code, 1)
        self.assertIn("未知", section(out, "Worker"))
        self.assertNotIn("没有漂移", section(out, "Worker"))
        # 没给 --mac 就去读本机的 LaunchAgent；这台没有，也是未知。
        code, out, _ = box.report(box.record(), mac=False)
        self.assertEqual(code, 1)
        self.assertIn("未知", section(out, "Mac 副本包"))
        # 169 上读不出来的那一项由记录自己说明，照样是未知。
        for key, part in (("human", "人侧程序"), ("agent", "agent"), ("alert", "告警包")):
            code, out, _ = box.report(box.record(**{key: {"error": "读不到发行目录"}}))
            self.assertEqual(code, 1, key)
            self.assertIn("未知：读不到发行目录", section(out, part))
            self.assertNotIn("没有漂移", section(out, part))
        # 随包工具可以没有：这个发行不带它，不算漂移，也不装作核过。
        human = box.human()
        del human["sync_program_source_commit"]
        code, out, _ = box.report(box.record(human=human))
        self.assertEqual(code, 0, out)
        self.assertIn("这个发行没有随包工具", section(out, "人侧随包工具"))

    def test_the_mac_package_is_read_from_the_launch_agent_and_checked_byte_for_byte(self):
        box = self.box
        package = box.home / ".local/share/studiolo-agent-sync" / f"t215-{box.base[:7]}-mac-studio"
        (package / "scripts").mkdir(parents=True)
        for name in ("sync-agent-vault.sh", "replica_common.py"):
            shutil.copy(box.repo / "scripts" / name, package / "scripts" / name)
        agents = box.home / "Library/LaunchAgents"
        agents.mkdir(parents=True)
        plist = ('<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>'
                 '<key>Label</key><string>me.studiolo.agent-sync</string><key>ProgramArguments</key><array>'
                 f'<string>/bin/bash</string><string>{package}/scripts/sync-agent-vault.sh</string></array></dict></plist>')
        (agents / "me.studiolo.agent-sync.plist").write_text(plist)
        code, out, _ = box.report(box.record(), mac=False)
        self.assertEqual(code, 0, out)
        self.assertIn("没有漂移", section(out, "Mac 副本包"))
        (package / "scripts/replica_common.py").write_text("改过\n")
        code, out, _ = box.report(box.record(), mac=False)
        self.assertEqual(code, 1)
        self.assertIn("replica_common.py 和这个提交里的不一样", section(out, "Mac 副本包"))
        (package / "scripts/replica_common.py").unlink()
        code, out, _ = box.report(box.record(), mac=False)
        self.assertEqual(code, 1)
        self.assertIn("和这个提交里的不一样", section(out, "Mac 副本包"))

    def test_bad_input_fails_closed(self):
        box = self.box
        good = box.record()
        missing = {key: value for key, value in good.items() if key != "agent"}
        short = box.record(human=box.human(web_source_commit=box.base[:7]))
        cases = {
            "不是 JSON": "{",
            "空文件": "",
            "不是对象": "[]",
            "重复键": json.dumps(good)[:-1] + ', "schema": 1}',
            "多一个键": {**good, "extra": 1},
            "人侧多一个键": box.record(human=box.human(extra="x")),
            "少一项": missing,
            "人侧少一项": box.record(human={key: value for key, value in good["human"].items()
                                            if key != "web_source_commit"}),
            "版本不认识": {**good, "schema": 2},
            "版本是真值": {**good, "schema": True},
            "人侧提交不是全长": short,
            "提交是数字": box.record(agent={"release": "x", "commit": 1}),
            "提交夹带参数": box.record(agent={"release": "x", "commit": "--output=/tmp/x"}),
            "提交大写": box.record(agent={"release": "x", "commit": box.base.upper()}),
            "提交不存在": box.record(agent={"release": "x", "commit": "0" * 39 + "1"}),
            "发布名带换行": box.record(agent={"release": "x\n## 假标题", "commit": box.base}),
            "错误不是文字": box.record(alert={"error": 1}),
            "错误带换行": box.record(alert={"error": "a\n## b"}),
            "超过大小": json.dumps({**good, "collected_at": "x" * 70000}),
            "不是 UTF-8": b"\xff\xfe",
            "空对象": {},
        }
        for name, record in cases.items():
            with self.subTest(name):
                code, out, err = box.report(record)
                self.assertEqual(code, 2, out)
                self.assertNotIn("没有漂移", out)
                self.assertNotIn("结论", out)
                self.assertTrue(err.startswith("deploy-drift: "), err)
                self.assertNotIn("Traceback", err)
        for name, extra in {"记录不存在": ["--record", str(box.root / "none.json")],
                            "main 不存在": ["--main", "origin/none"],
                            "main 夹带参数": ["--main=--all"],
                            "06-BUG 不存在": ["--bugs", str(box.root / "none")],
                            "仓库不是仓库": ["--repo", str(box.bugs)],
                            "Worker 提交不存在": ["--worker", "0" * 40],
                            "Mac 提交写错": ["--mac", "HEAD"]}.items():
            with self.subTest(name):
                code, out, err = box.report(good, *extra)
                self.assertEqual(code, 2, out)
                self.assertNotIn("结论", out)
                self.assertNotIn("Traceback", err)

    def test_a_program_whose_package_cannot_be_found_fails_closed(self):
        box = self.box
        base = box.git("rev-parse", "HEAD")
        for path in ("agent-server", "supply"):
            shutil.rmtree(box.repo / "crates" / path)
        box.git("add", "-A")
        box.git("commit", "-q", "-m", "删掉 agent 的包")
        gone = box.git("rev-parse", "HEAD")
        # 线上那份还有这个包：照旧对账，删包本身也是没上线的改动。
        code, out, _ = box.report(box.record())
        self.assertEqual(code, 1)
        self.assertIn("- " + gone[:7], section(out, "agent"))
        # 两头都没有：闭包是空的，对不出来，不许说没有漂移。
        self.assertNotEqual(base, gone)
        human = box.human(human_program_source_commit=gone, web_source_commit=gone, sync_program_source_commit=gone)
        record = box.record(human=human, agent={"release": "x", "commit": gone}, alert={"commit": gone})
        code, out, err = box.report(record, mac=gone, worker=gone)
        self.assertEqual((code, out), (2, ""))
        self.assertIn("studiolo-agent-server", err)

    def test_bugs_whose_fix_is_live_are_listed_apart(self):
        box = self.box
        live = box.touch("fix: 已上线", "crates/auth/src/lib.rs")
        pending = box.touch("fix: 没上线", "crates/auth/src/lib.rs")
        box.git("checkout", "-q", "-b", "side")
        branch = box.touch("fix: 分支上的", "crates/auth/src/lib.rs")
        box.git("checkout", "-q", "main")
        tests_only = box.touch("test: 只改测试", "crates/auth/tests/x.rs")
        other = box.touch("fix: 备份 worker", "crates/backup-worker/src/lib.rs")
        box.bug(1, f"修复中   # 代码已上线：`{live[:7]}` 合 main（发布名 `t1-{live[:7]}-x`）")
        box.bug(2, f"修复中   # `{live[:7]}`、`{pending[:7]}` 合 main")
        box.bug(3, "修复中   # 未修复 / 修复中 / 已修复", "还没写提交")
        box.bug(4, f"修复中   # `{branch[:7]}`")
        box.bug(5, f"已修复   # `{live[:7]}`")
        box.bug(6, "修复中", f"基线 main `{pending[:7]}`，没合 main 的 `{branch[:7]}`。\n代码已合 main `{live[:7]}`。")
        box.bug(7, f"修复中   # `{tests_only[:7]}`")
        box.bug(8, f"修复中   # `{live[:7]}`、`{other[:7]}`")
        box.bug(9, "修复中   # `deadbeef`")
        box.bug(10, f"修复中   # `{live[:7]}`、`{tests_only[:7]}`，任务 `20261118`、ci `20261125-102751-bc2f`")
        box.bug(11, f"修复中   # `{box.base[:7]}..{live[:7]}`")
        box.bug(12, f"修复中   # `{live[:7]}..{pending[:7]}`")
        box.bug(13, f"未修复   # `{live[:7]}`")
        (box.bugs / "别的笔记.md").write_text("---\n状态: 修复中\n---\n")
        record = box.record(human=box.human(human_program_source_commit=live, sync_program_source_commit=live))
        code, out, _ = box.report(record)
        self.assertEqual(code, 1, out)
        ready = section(out, "该验收了")
        waiting = section(out, "修复中、还没上线")
        manual = section(out, "修复中、要人看")
        listed = [set(map(int, re.findall(r"^- BUG-0*(\d+)", text, re.M))) for text in (ready, waiting, manual)]
        self.assertEqual(listed, [{1, 6, 10, 11}, {2, 12}, {3, 4, 7, 8, 9}], out)
        self.assertIn(pending[:7], waiting)
        self.assertIn("人侧程序", waiting)
        for number, reason in ((3, "看不出修复提交"), (4, "不在 main"), (7, "没有碰五个发行"), (8, "不归这五个发行"),
                               (9, "没有这个提交")):
            self.assertRegex(manual, f"BUG-{number:03d}.*{reason}")

    def test_a_bug_waiting_for_acceptance_alone_is_worth_a_nonzero_exit(self):
        box = self.box
        fix = box.touch("fix: 已上线", "crates/auth/src/lib.rs")
        box.bug(1, f"修复中   # `{fix[:7]}`")
        human = box.human(human_program_source_commit=fix, sync_program_source_commit=fix)
        code, out, _ = box.report(box.record(human=human))
        self.assertEqual(code, 1, out)
        self.assertIn("结论：五个发行都没有漂移，有 1 个 BUG 该验收了。", out)
        self.assertIn("标题1", section(out, "该验收了"))

    def test_a_bug_cannot_be_ready_when_a_release_it_touches_is_unknown(self):
        box = self.box
        fix = box.touch("fix: 心跳", "deploy/alerts/worker/src/worker.mjs")
        box.bug(1, f"修复中   # `{fix[:7]}`")
        _, out, _ = box.report(box.record(), worker=False)
        self.assertNotIn("BUG-001", section(out, "该验收了"))
        self.assertRegex(section(out, "修复中、要人看"), "BUG-001.*Worker.*未知")
        _, out, _ = box.report(box.record(), worker=fix)
        self.assertIn("BUG-001", section(out, "该验收了"))

    def test_paths_the_script_does_not_know_are_shown(self):
        box = self.box
        box.touch("新的一块", "services/new/main.rs")
        code, out, _ = box.report(box.record())
        self.assertEqual(code, 1)
        self.assertIn("services/new/main.rs", section(out, "没归类的路径"))

    def test_it_never_reaches_the_network_and_never_writes(self):
        box = self.box
        box.touch("fix", "crates/auth/src/lib.rs")
        box.bug(1, f"修复中   # `{box.base[:7]}`")
        before = {path: path.read_bytes() for path in sorted(box.root.rglob("*"))
                  if path.is_file() and path.name != "record.json"}
        box.report(box.record())
        box.run("command")
        box.run("probe")
        after = {path: path.read_bytes() for path in sorted(box.root.rglob("*"))
                 if path.is_file() and path.name != "record.json"}
        self.assertEqual(set(before), set(after))
        self.assertEqual(before, after)
        self.assertFalse(box.called.exists())


class CommandTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.box = Sandbox(directory.name)

    def test_the_169_step_is_printed_as_one_read_only_command_without_sudo(self):
        code, out, _ = self.box.run("command")
        self.assertEqual(code, 0)
        commands = [line for line in out.splitlines() if line and not line.startswith("#")]
        self.assertEqual(len(commands), 2)
        self.assertRegex(commands[0], r"^python3 scripts/deploy-drift\.py probe \| ssh studiolo-169 "
                                      r"'python3 -I -B -S -' > \S+\.json$")
        self.assertTrue(commands[1].startswith("python3 scripts/deploy-drift.py report --record "))
        self.assertEqual(commands[0].split("> ")[1], commands[1].split("--record ")[1])
        self.assertNotIn("sudo", out)
        _, out, _ = self.box.run("command", "--host", "maxsong@192.168.100.169")
        self.assertIn("ssh maxsong@192.168.100.169 ", out)
        for bad in ("-oProxyCommand=x", "a b", "a;b", "", "a'b"):
            code, out, _ = self.box.run("command", "--host=" + bad)
            self.assertEqual((code, out), (2, ""), bad)

    def test_the_probe_only_reads(self):
        code, out, _ = self.box.run("probe")
        self.assertEqual(code, 0)
        self.assertEqual(out, drift.PROBE)
        for word in ("sudo", "'w'", '"w"', "write_", "unlink", "remove", "rmtree", "rename", "mkdir", "chmod", "kill",
                     "start'", "stop'", "restart", "socket", "urllib"):
            self.assertNotIn(word, out, word)

    def test_the_probe_runs_on_a_machine_with_nothing_installed(self):
        result = subprocess.run([sys.executable, "-I", "-B", "-S", "-"], input=drift.PROBE, capture_output=True, text=True,
                                timeout=60, env={"PATH": str(self.box.bin), "LC_ALL": "C.UTF-8"})
        self.assertEqual(result.returncode, 0, result.stderr)
        value = json.loads(result.stdout)
        for key in ("human", "agent", "alert"):
            self.assertEqual(set(value[key]), {"error"}, value)
        # 这份「什么都没读到」的记录喂给对账：三个都是未知，不是没漂移。
        code, out, _ = self.box.report(value)
        self.assertEqual(code, 1)
        for part in ("人侧程序", "人侧网页", "人侧随包工具", "agent", "告警包"):
            self.assertIn("未知", section(out, part))


class ProbeTests(unittest.TestCase):
    """探针在假的根目录上：单元指到哪个发行就读哪个，读到什么算什么。"""

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.box = Sandbox(directory.name)
        self.root = Path(directory.name) / "remote"
        self.human = "/home/maxsong/.local/share/studiolo-human-releases/m5m6-board-20260920-r27"
        self.agent = "/home/maxsong/.local/share/studiolo-agent-releases/t215-116f5d7-20261120"
        base = self.box.base
        self.put(self.human + "/manifest.json", json.dumps({
            "build": "r27", "bootstrap_source_commit": base, "human_program_source_commit": base,
            "web_source_commit": base, "sync_program_source_commit": base, "files": {}, "assembly": "x"}))
        self.put(self.agent + "/base-commit.txt", base + "\n")
        self.put(self.agent + "/provenance.txt", "来源：与 base-commit 一致的干净工作区快照（白名单内无未提交改动）。\n")
        releases = self.root / "opt/studiolo-alert/releases" / base
        releases.mkdir(parents=True)
        (self.root / "opt/studiolo-alert/current").symlink_to("/opt/studiolo-alert/releases/" + base)
        self.units = {
            "studiolo-human.service": "{ path=/usr/bin/python3 ; argv[]=/usr/bin/python3 -I -B -S "
                                      + self.human + "/launch-human.py ; ignore_errors=no }",
            "studiolo-agent.service": "{ path=" + self.agent + "/bin/studiolo-agent ; argv[]=" + self.agent
                                      + "/bin/studiolo-agent serve --replica-root /home/x }",
        }
        namespace = {"__name__": "probe"}
        exec(compile(drift.PROBE, "probe", "exec"), namespace)
        self.collect = namespace["collect"]

    def put(self, path, body):
        target = self.root / path.lstrip("/")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(body)

    def value(self):
        return self.collect(str(self.root), lambda unit: self.units.get(unit, ""), "2026-11-28T10:00:00Z")

    def test_what_it_collects_is_what_the_report_accepts(self):
        value = self.value()
        self.assertEqual(value["human"]["release"], "m5m6-board-20260920-r27")
        self.assertEqual(value["agent"], {"release": "t215-116f5d7-20261120", "commit": self.box.base})
        self.assertEqual(value["alert"], {"commit": self.box.base})
        code, out, err = self.box.report(json.loads(json.dumps(value)))
        self.assertEqual(code, 0, out + err)

    def test_a_manifest_without_tools_is_passed_on_as_it_is(self):
        path = self.root / (self.human.lstrip("/") + "/manifest.json")
        value = json.loads(path.read_text())
        del value["sync_program_source_commit"]
        path.write_text(json.dumps(value))
        self.assertNotIn("sync_program_source_commit", self.value()["human"])
        self.assertEqual(self.box.report(self.value())[0], 0)

    def test_an_agent_built_from_a_dirty_tree_has_no_source_commit(self):
        self.put(self.agent + "/provenance.txt", "来源：当前工作区文件快照（包含未提交改动）；base-commit 不是本运行包的版本标识。\n")
        self.assertEqual(set(self.value()["agent"]), {"error"})

    def test_anything_it_cannot_read_becomes_an_error_for_that_release_only(self):
        for change, key in (
                (lambda: self.units.pop("studiolo-human.service"), "human"),
                (lambda: self.units.update({"studiolo-agent.service": "{ path=/usr/bin/env }"}), "agent"),
                (lambda: (self.root / (self.agent.lstrip("/") + "/base-commit.txt")).unlink(), "agent"),
                (lambda: self.put(self.human + "/manifest.json", "{"), "human"),
                (lambda: self.put(self.human + "/manifest.json", "[]"), "human"),
                (lambda: self.put(self.human + "/manifest.json", "{}"), "human"),
                (lambda: self.put(self.agent + "/base-commit.txt", "main\n"), "agent"),
                (lambda: (self.root / "opt/studiolo-alert/current").unlink(), "alert")):
            with self.subTest(key=key):
                self.setUp()
                change()
                value = self.value()
                self.assertEqual(set(value[key]), {"error"}, value)
                for other in {"human", "agent", "alert"} - {key}:
                    self.assertNotIn("error", value[other])


class CoverageTests(unittest.TestCase):
    """拿真仓库核路径归类：每个文件都归得到一处，程序内嵌的仓库外文件一个不漏。"""

    def files(self):
        result = []
        skip = {".git", "target", "node_modules", "dist", ".build", "DerivedData", "__pycache__", ".claude"}
        for directory, names, files in os.walk(WORKSPACE):
            names[:] = [name for name in names if name not in skip]
            base = os.path.relpath(directory, WORKSPACE)
            result += [name if base == "." else f"{base}/{name}" for name in files]
        return result

    def test_every_tracked_path_has_a_home(self):
        manifests = {path: (WORKSPACE / path).read_text() for path in self.files()
                     if re.fullmatch(r"crates/[^/]+/Cargo\.toml", path)}
        parts = drift.parts_for([manifests])
        lost = [path for path in self.files()
                if path.split("/")[0] in {"crates", "deploy", "scripts", "web", "docs", "examples", "bottega"}
                and drift.home(path, parts) is None]
        self.assertEqual(lost, [])

    def test_files_embedded_from_outside_a_crate_belong_to_its_release(self):
        found = set()
        for path in self.files():
            if not re.match(r"crates/[^/]+/src/.*\.rs$", path):
                continue
            for target in re.findall(r'include_(?:str|bytes)!\(\s*"([^"]+)"', (WORKSPACE / path).read_text()):
                resolved = os.path.normpath(os.path.join(os.path.dirname(path), target))
                if resolved.split("/")[:2] != path.split("/")[:2] and not drift.is_test(path):
                    found.add((path.split("/")[1], resolved))
        declared = {(crate, path) for crate, paths in drift.EMBEDDED.items() for path in paths}
        self.assertEqual(found - declared, set())

    def test_the_closure_of_each_program_is_what_cargo_says(self):
        manifests = {path: (WORKSPACE / path).read_text() for path in self.files()
                     if re.fullmatch(r"crates/[^/]+/Cargo\.toml", path)}
        agent = drift.closure(manifests, ["studiolo-agent-server"])
        self.assertIn("supply", agent)
        self.assertNotIn("auth", agent)
        self.assertNotIn("local-server", agent)
        human = drift.closure(manifests, ["studiolo-local-server"])
        self.assertTrue({"auth", "vault-broker", "backup", "sync-bridge", "runner-adapter"} <= human)
        self.assertNotIn("backup-worker", human)


if __name__ == "__main__":
    unittest.main()
