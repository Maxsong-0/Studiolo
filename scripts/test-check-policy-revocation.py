#!/usr/bin/env python3
"""隔离 SQLite + 假 Client 的 fixture 吊销验收测试；不读取真实路径，不绑定 4401。"""
import contextlib
from email.message import Message
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import sqlite3
import sys
import tempfile
import unittest
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("check_policy_revocation",
    Path(__file__).resolve().parents[1] / "deploy/agent/check-policy-revocation.py")
rev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rev)
TOKEN, SESSION, PRIVATE = "fixture-token-private", "fixture-session-private", "fixture-note-private"
HASH = "0123456789ab"
SCHEMA = """
PRAGMA application_id=1397772108;
PRAGMA user_version=1;
CREATE TABLE policy_revocation (
  seq INTEGER PRIMARY KEY, level TEXT NOT NULL, target TEXT NOT NULL,
  revoked_at TEXT NOT NULL, revoked_by TEXT NOT NULL, reason TEXT NOT NULL,
  released_at TEXT, released_by TEXT);
CREATE UNIQUE INDEX revocation_active ON policy_revocation(level,target) WHERE released_at IS NULL;
"""


def signpost(instance):
    return ("# Studiolo 路牌（代次 1，2026-09-20）\n## 1 我是谁\n"
            f"类型 claude · 实例 {instance} · 注册 2026-09-20\n## 2 项目状态\n{PRIVATE}\n"
            "## 3 能力\n读：项目/Studiolo（能力集 v1 ∩ 部署上限）；写：黑板 无（M5）、正文 "
            f"仅 patch（M5 起）；策略：可提 desired（永不自动生效）；策略版本 {HASH}\n"
            "## 4 待我做的事\n无\n## 5 硬约束\n无\n## 6 继续拉的入口\npolicy_read\n")


class FakeClient:
    def __init__(self, fixture, base, token):
        assert base == "http://127.0.0.1:4401/mcp" and token == TOKEN
        self.fixture, self.responses, self.session = fixture, 0, None
        fixture.starts += 1
        self.instance = f"{fixture.starts:026d}"

    def denied(self):
        with sqlite3.connect(self.fixture.db) as db:
            for level, target in (("global", "*"), ("type", "claude"), ("instance", self.instance)):
                if db.execute("SELECT 1 FROM policy_revocation WHERE level=? AND target=? "
                              "AND released_at IS NULL", (level, target)).fetchone():
                    return level
        return None

    def rpc(self, request_id, method, params, allow_error=False):
        self.responses += 1
        headers = Message()
        headers["Mcp-Session-Id"] = SESSION
        denied = self.denied()
        if denied and self.fixture.interrupt:
            raise OSError(TOKEN + PRIVATE)
        response = {"jsonrpc": "2.0", "id": request_id}
        if method == "initialize":
            response["result"] = {"protocolVersion": rev.probe.agent.PROTOCOL,
                                  "serverInfo": {"name": "studiolo-agent"},
                                  "instructions": rev.denial_text(denied) if denied else signpost(self.instance)}
            self.fixture.initialized.append((self.instance, denied))
        elif method == "tools/list":
            response["result"] = {"tools": [{"name": name} for name in rev.probe.agent.EXPECTED_TOOLS]}
        else:
            assert method == "tools/call"
            name, args = params["name"], params["arguments"]
            assert name in {"handshake", "policy_read", "read_note"}
            self.fixture.calls.append((self.instance, name, denied))
            if denied:
                response["error"] = {"code": -32600, "message": rev.denial_text(denied)}
            elif name == "read_note" and args["path"] == "School/x.md":
                response["error"] = {"code": -32602, "message": "路径不在供给白名单内"}
            else:
                text = (signpost(self.instance) if name == "handshake" else
                        f"# 策略版本 {HASH}\n0 个 skill（索引式候选；硬绑定见路牌第 5 项）\n"
                        if name == "policy_read" else f"revision {'0' * 64}\n# Studiolo · 总览\n{PRIVATE}")
                response["result"] = {"content": [{"type": "text", "text": text}]}
        if not allow_error:
            rev.require("error" not in response, "假服务拒绝只读调用")
        return headers, response

    def post(self, body, expected=200):
        assert body["method"] == "notifications/initialized" and expected == 202
        self.responses += 1
        return Message(), ""

    def call(self, request_id, name, arguments):
        return self.rpc(request_id, "tools/call", {"name": name, "arguments": arguments})[1]["result"]["content"][0]["text"]


class FixtureTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="policy-revocation-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve() / "studiolo-m4-stage-t087-Test123"
        self.root.mkdir(mode=0o700)
        for name in ("policy", "config"):
            (self.root / name).mkdir(mode=0o700)
        self.db = self.root / "policy/policy.sqlite3"
        with sqlite3.connect(self.db) as db:
            db.executescript(SCHEMA)
            db.execute("INSERT INTO policy_revocation VALUES (1,'instance','unrelated','before',"
                       "'owner','preserve',NULL,NULL)")
            db.execute("INSERT INTO policy_revocation VALUES (2,'type','claude','before',"
                       "'owner','preserve','released','owner')")
        self.db.chmod(0o600)
        self.token = self.root / "config/claude.token"
        self.token.write_text(TOKEN)
        self.token.chmod(0o600)
        self.production = Path(self.temp.name).resolve() / "production-fixture.sqlite3"
        self.production.write_bytes(b"not a real production database")
        self.production.chmod(0o600)
        self.original = self.rows()
        self.starts, self.calls, self.initialized, self.interrupt = 0, [], [], False

    def rows(self):
        with sqlite3.connect(self.db) as db:
            return db.execute(f"SELECT {rev.COLUMNS} FROM policy_revocation ORDER BY seq").fetchall()

    @contextlib.contextmanager
    def isolated_paths(self, pattern=None):
        with mock.patch.object(rev, "STAGE_PATTERN", pattern or re.compile(re.escape(str(self.root)))), \
                mock.patch.object(rev, "PRODUCTION_DB", self.production):
            yield

    def run_check(self):
        output = io.StringIO()
        with self.isolated_paths(), mock.patch.object(rev.probe, "Client",
                side_effect=lambda base, token: FakeClient(self, base, token)), \
                contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            code = rev.main([str(self.root)])
        text = output.getvalue()
        for secret in (TOKEN, SESSION, PRIVATE):
            self.assertNotIn(secret, text)
        return code, text

    def test_three_levels_apply_to_running_sessions_and_only_own_rows_are_cleaned(self):
        code, output = self.run_check()
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(output), {"fixture_levels_checked": 3, "existing_session_denials": 3,
            "fresh_session_denials": 2, "unrelated_instances_allowed": 1,
            "fixture_rows_cleaned": 3, "effective_hash_prefix": HASH})
        self.assertEqual(self.rows(), self.original)
        self.assertEqual([denial for _, denial in self.initialized if denial], ["type", "global"])
        for level in ("instance", "type", "global"):
            self.assertTrue(any(name == "read_note" and denial == level for _, name, denial in self.calls))

    def test_mid_check_failure_cleans_fixture_and_sanitizes_exception(self):
        self.interrupt = True
        code, output = self.run_check()
        self.assertEqual(code, 1)
        self.assertTrue(output.startswith("阶段吊销验收失败："))
        self.assertEqual(len(output.splitlines()), 1)
        self.assertEqual(self.rows(), self.original)

    def test_exact_production_path_gate_rejects_other_paths(self):
        valid = "/home/maxsong/.local/share/studiolo-m4-stage-t087-AbC123"
        self.assertIsNotNone(rev.STAGE_PATTERN.fullmatch(valid))
        for value in (str(self.root), str(rev.PRODUCTION_DB.parent), valid + "/", valid + "/../production",
                      valid + "-suffix", valid + "\n", "/tmp/" + Path(valid).name):
            self.assertIsNone(rev.STAGE_PATTERN.fullmatch(value))
            with self.assertRaises(rev.probe.agent.CheckFailed):
                rev.validate_stage(value)

    def test_same_inode_and_nonprivate_directories_are_rejected(self):
        with self.isolated_paths(), mock.patch.object(rev, "PRODUCTION_DB", self.db):
            with self.assertRaises(rev.probe.agent.CheckFailed):
                rev.validate_stage(str(self.root))
        for path in (self.root, self.root / "policy", self.root / "config"):
            with self.subTest(path=path.name), self.isolated_paths():
                path.chmod(0o755)
                try:
                    with self.assertRaises(rev.probe.agent.CheckFailed):
                        rev.validate_stage(str(self.root))
                finally:
                    path.chmod(0o700)

    def test_symlink_ancestor_token_and_sidecar_are_rejected(self):
        alias = self.root.parent / "alias"
        alias.symlink_to(self.root, target_is_directory=True)
        for target in (alias, alias / "child"):
            with self.isolated_paths(re.compile(re.escape(str(target)))):
                with self.assertRaises(rev.probe.agent.CheckFailed):
                    rev.validate_stage(str(target))
        saved = self.token.with_name("saved-token")
        self.token.rename(saved)
        self.token.symlink_to(saved)
        with self.isolated_paths():
            with self.assertRaises(rev.probe.agent.CheckFailed):
                rev.validate_stage(str(self.root))
        self.token.unlink()
        saved.rename(self.token)
        sidecar = Path(str(self.db) + "-wal")
        sidecar.symlink_to(self.production)
        with self.isolated_paths():
            with self.assertRaises(rev.probe.agent.CheckFailed):
                rev.validate_stage(str(self.root))

    def test_preexisting_target_and_changed_test_row_are_never_deleted(self):
        with sqlite3.connect(self.db) as db:
            with self.assertRaises(rev.probe.agent.CheckFailed):
                with rev.fixture_revocation(db, "instance", "unrelated"):
                    self.fail("existing revocation must not be overwritten")
            self.assertEqual(self.rows(), self.original)
            with self.assertRaises(rev.probe.agent.CheckFailed):
                with rev.fixture_revocation(db, "global", "*"):
                    db.execute("UPDATE policy_revocation SET reason='externally changed' WHERE level='global'")
                    db.commit()
            self.assertEqual(db.execute("SELECT reason FROM policy_revocation WHERE level='global'").fetchone(),
                             ("externally changed",))
            self.assertEqual(self.rows()[:2], self.original)

    def test_exception_inside_fixture_always_cleans_only_own_row(self):
        with sqlite3.connect(self.db) as db:
            with self.assertRaisesRegex(RuntimeError, "fixture failure"):
                with rev.fixture_revocation(db, "type", "claude"):
                    raise RuntimeError("fixture failure")
        self.assertEqual(self.rows(), self.original)


if __name__ == "__main__":
    unittest.main()
