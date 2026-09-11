#!/usr/bin/env python3
"""HTTP 投影采集协议及客户端输出边界；全用内存响应，不调用远端服务。"""
import io
import json
import sys
import unittest
import urllib.error
import urllib.parse
from email.message import Message
from unittest.mock import patch

sys.dont_write_bytecode = True
import client_projection_fetch as fetcher
from client_projection_render import render
from replica_common import Invalid, digest

NOW = 1_789_056_579_000
INSTANCE = "01K00000000000000000000000"
SESSION = "01K11111111111111111111111"
TOKEN = "PUBLIC_TEST_ONLY_CANARY"


class Response(io.BytesIO):
    def __init__(self, target, body, headers):
        super().__init__(body.encode("utf-8") if isinstance(body, str) else body)
        self.status, self.target, self.headers = 200, target, headers

    def geturl(self):
        return self.target


class Fixture:
    def __init__(self):
        self.config = {"schema_version": 1, "client": "claude", "project": "项目/Alpha",
                       "source_url": "https://example.invalid/read", "token_env": "TEST_TOKEN",
                       "rules": ["_规则/准则.md", "项目/Alpha/CLAUDE.md"],
                       "skills": ["_规则/skills/explain"]}
        self.notes = {"_规则/准则.md": "规则正文\n", "项目/Alpha/CLAUDE.md": "项目规则\n",
                      "_规则/skills/explain/SKILL.md": "---\nname: explain\n---\n参考 [详情](resources/详情.md)\n",
                      "_规则/skills/explain/resources/详情.md": "\ufeff原文\r\n保留字节\r\n"}
        self.extra, self.calls, self.mutate = [], [], lambda route, text, headers: text
        self.blank_line = True

    def fresh(self):
        return ("副本状态 ready；stale=false；generation=" + "a" * 32 + "；source_digest=" + "b" * 64
                + f"；source_observed_at_ms={NOW - 1000}；received_at_ms={NOW - 500}；age_secs=1；max_age_secs=300")

    def open(self, request, timeout):
        self.calls.append(request)
        if timeout != 15:
            raise AssertionError("必须设置十五秒超时")
        if request.get_header("Authorization") != "Bearer " + TOKEN:
            raise AssertionError("所有读取必须认证")
        if len(self.calls) > 1 and request.get_header("Studiolo-session-id") != SESSION:
            raise AssertionError("续读必须复用同一会话")
        parsed = urllib.parse.urlsplit(request.full_url)
        route, query = parsed.path.rsplit("/", 1)[1], urllib.parse.parse_qs(parsed.query)
        headers = Message()
        headers["Content-Type"] = "text/plain; charset=utf-8"
        if route == "handshake":
            headers["Studiolo-Session-Id"], headers["Studiolo-Instance-Id"] = SESSION, INSTANCE
            text = ("# Studiolo 路牌（代次 1，时间）\n" + self.fresh()
                    + ("\n\n" if self.blank_line else "\n") + "## 1 我是谁\n"
                    + f"类型 {self.config['client']} · 实例 {INSTANCE} · 注册 时间\n"
                    + "其他项目绝密路牌，不得写入投影\n")
        elif route == "entries":
            prefix = query["project"][0]
            rows = [f"note\t{p}\t{len(body.encode('utf-8'))}" for p, body in self.notes.items()
                    if p.startswith(prefix + "/")]
            if prefix == "_规则":
                rows += ["dir\t_规则/skills\t", "dir\t_规则/skills/explain\t",
                         "dir\t_规则/skills/explain/resources\t"] + self.extra
            text = self.fresh() + f"\n# 代次 1 · {len(rows)} 条\n" + "\n".join(rows)
        else:
            body = self.notes[query["path"][0]]
            # 故意使用不同于正文 SHA 的 revision，匹配实际 Vault 域分离协议。
            text = "revision " + "c" * 64 + "\n" + self.fresh() + "\n" + body
        return Response(request.full_url, self.mutate(route, text, headers), headers)

    def fetch(self):
        with patch.object(fetcher.urllib.request, "build_opener", return_value=self), \
                patch.object(fetcher.time, "time", return_value=NOW / 1000):
            return fetcher.fetch(self.config, TOKEN)


class FetchTests(unittest.TestCase):
    def setUp(self):
        self.fixture = Fixture()

    def rejected(self):
        with self.assertRaises(Invalid) as error:
            self.fixture.fetch()
        self.assertEqual(str(error.exception), "projection_fetch_failed")
        self.assertNotIn(TOKEN, str(error.exception))

    def test_exact_sources_and_raw_bytes_with_domain_separated_revision(self):
        snapshot = self.fixture.fetch()
        self.assertEqual(snapshot["identity"], {"type": "claude", "instance": INSTANCE})
        self.assertEqual(len(snapshot["sources"]), 4)
        for source in snapshot["sources"]:
            self.assertEqual(source["body"], self.fixture.notes[source["path"]])
            self.assertNotEqual(source["revision"], digest(source["body"].encode()))
        self.assertNotIn("其他项目绝密", json.dumps(snapshot, ensure_ascii=False))
        self.assertNotIn(SESSION, json.dumps(snapshot))
        self.assertNotIn(TOKEN, json.dumps(snapshot))
        self.assertTrue(self.fixture.calls[-1].full_url.endswith("/handshake"))

    def test_fixed_identity_position_allows_zero_or_one_blank_line_only(self):
        for blank in (False, True):
            self.fixture = Fixture()
            self.fixture.blank_line = blank
            self.assertEqual(self.fixture.fetch()["identity"]["instance"], INSTANCE)
        self.fixture = Fixture()
        self.fixture.mutate = lambda route, text, h: text.replace("\n\n## 1 我是谁", "\n\n\n## 1 我是谁")
        self.rejected()

    def test_stale_failed_missing_and_false_age_are_rejected(self):
        for before, after in (("副本状态 ready", "副本状态 stale"), ("stale=false", "stale=true"),
                              ("副本状态 ready", "副本状态 sync_failed"), ("age_secs=1；", "age_secs=400；"),
                              ("source_observed_at_ms=", "missing=")):
            with self.subTest(after=after):
                self.fixture = Fixture()
                self.fixture.mutate = lambda route, text, h: text.replace(before, after)
                self.rejected()

    def test_locally_expired_timestamp_is_rejected_even_when_server_says_ready(self):
        self.fixture.mutate = lambda route, text, h: text.replace(str(NOW - 1000), str(NOW - 400_000))
        self.rejected()

    def test_source_change_mid_batch_and_at_final_handshake_is_rejected(self):
        for changed in ("note", "final"):
            with self.subTest(changed=changed):
                self.fixture = Fixture()
                def change(route, text, headers):
                    if route == changed or (changed == "final" and route == "handshake" and len(self.fixture.calls) > 1):
                        return text.replace("generation=" + "a" * 32, "generation=" + "d" * 32)
                    return text
                self.fixture.mutate = change
                self.rejected()

    def test_body_cannot_supply_missing_metadata(self):
        def change(route, text, headers):
            return text.replace(self.fixture.fresh(), "无效元数据", 1) + "\n" + self.fixture.fresh()
        self.fixture.mutate = change
        self.rejected()

    def test_confirmation_can_advance_without_changing_immutable_sources(self):
        def change(route, text, headers):
            if len(self.fixture.calls) > 1:
                return text.replace(str(NOW - 1000), str(NOW - 800)).replace(str(NOW - 500), str(NOW - 400))
            return text
        self.fixture.mutate = change
        snapshot = self.fixture.fetch()
        self.assertEqual(snapshot["freshness"]["source_observed_at_ms"], NOW - 1000)
        self.assertEqual(snapshot["freshness"]["received_at_ms"], NOW - 500)

    def test_confirmation_cannot_regress(self):
        def change(route, text, headers):
            return text.replace(str(NOW - 1000), str(NOW - 1200)) if len(self.fixture.calls) > 1 else text
        self.fixture.mutate = change
        self.rejected()

    def test_actual_identity_type_does_not_depend_on_output_format(self):
        self.fixture.mutate = lambda route, text, h: text.replace("类型 claude", "类型 fixture")
        self.assertEqual(self.fixture.fetch()["identity"]["type"], "fixture")

    def test_identity_and_session_must_remain_bound(self):
        for name in ("Studiolo-Instance-Id", "Studiolo-Session-Id"):
            with self.subTest(name=name):
                self.fixture = Fixture()
                def change(route, text, headers):
                    if route == "handshake" and len(self.fixture.calls) > 1:
                        headers.replace_header(name, "01K22222222222222222222222")
                    return text
                self.fixture.mutate = change
                self.rejected()

    def test_duplicate_header_is_rejected(self):
        def change(route, text, headers):
            headers["Content-Type"] = "text/plain; charset=utf-8"
            return text
        self.fixture.mutate = change
        self.rejected()

    def test_outside_root_duplicate_and_attachment_skill_entries_are_rejected(self):
        for row in ("note\t项目/Beta/private.md\t1", "note\t_规则/准则.md\t1",
                    "attachment\t_规则/skills/explain/resources/file.pdf\t1",
                    "note\t_规则/skills/explain/resources/file.py\t1"):
            with self.subTest(row=row):
                self.fixture = Fixture()
                self.fixture.extra = [row]
                self.rejected()

    def test_missing_skill_entrypoint_or_requested_rule_is_rejected(self):
        for path in ("_规则/skills/explain/SKILL.md", "_规则/准则.md"):
            with self.subTest(path=path):
                self.fixture = Fixture()
                del self.fixture.notes[path]
                self.rejected()

    def test_note_revision_and_size_must_match_protocol(self):
        for replacement in ("revision invalid", "revision " + "c" * 64 + "\n额外行"):
            self.fixture = Fixture()
            self.fixture.mutate = lambda route, text, h: text.replace("revision " + "c" * 64, replacement)
            self.rejected()
        self.fixture = Fixture()
        self.fixture.mutate = lambda route, text, h: text + "tampered" if route == "note" else text
        self.rejected()

    def test_source_count_and_single_note_size_are_bounded(self):
        with patch.object(fetcher, "MAX_SOURCES", 3):
            self.rejected()
        self.fixture = Fixture()
        self.fixture.notes["_规则/准则.md"] = "x" * (fetcher.MAX_NOTE + 1)
        self.rejected()

    def test_transport_error_never_exposes_url_or_secret(self):
        with patch.object(fetcher.urllib.request, "build_opener", side_effect=urllib.error.URLError(TOKEN)):
            with self.assertRaisesRegex(Invalid, "^projection_fetch_failed$"):
                fetcher.fetch(self.fixture.config, TOKEN)

    def test_note_cannot_export_current_token_or_session(self):
        for secret in (TOKEN, SESSION):
            with self.subTest(secret_kind="token" if secret == TOKEN else "session"):
                self.fixture = Fixture()
                self.fixture.notes["_规则/准则.md"] += secret
                self.rejected()

    def test_proxy_disabled_and_redirect_refused(self):
        with patch.object(fetcher.urllib.request, "build_opener") as build:
            fetcher.Client(self.fixture.config, TOKEN)
        handlers = build.call_args.args
        self.assertEqual(handlers[0].proxies, {})
        self.assertIsInstance(handlers[1], fetcher.NoRedirect)
        self.assertIsNone(handlers[1].redirect_request(None))

    def test_render_single_client_preserves_sources_and_uses_env_only(self):
        for client in ("claude", "codex"):
            self.fixture = Fixture()
            self.fixture.config["client"] = client
            snapshot = self.fixture.fetch()
            files = render(self.fixture.config, snapshot)
            for path, body in self.fixture.notes.items():
                self.assertEqual(files["sources/" + path], body.encode())
            if client == "claude":
                self.assertIn(b"@CONTEXT.md", files["CLAUDE.md"])
                self.assertNotIn("AGENTS.md", files)
                mcp = json.loads(files[".mcp.json"])["mcpServers"]["studiolo"]
                self.assertEqual(mcp["headers"]["Authorization"], "Bearer ${TEST_TOKEN}")
                self.assertEqual(mcp["url"], "https://example.invalid/mcp")
            else:
                self.assertNotIn("CLAUDE.md", files)
                self.assertIn(b'bearer_token_env_var = "TEST_TOKEN"', files[".codex/config.toml"])
            self.assertNotIn("manifest.json", files)
            self.assertNotIn(TOKEN.encode(), b"".join(files.values()))
            snapshot["generated_at_ms"] += 60_000
            snapshot["identity"]["instance"] = "01K22222222222222222222222"
            self.assertEqual(render(self.fixture.config, snapshot), files)


if __name__ == "__main__":
    unittest.main()
