#!/usr/bin/env python3
"""HTTP 投影采集协议及客户端输出边界；全用内存响应，不调用远端服务。"""
import json
import sys
import unittest
import urllib.error
import urllib.parse
from unittest.mock import patch

sys.dont_write_bytecode = True
import client_projection_fetch as fetcher
from client_projection_render import render
from client_projection_schema import make_bundle
from client_projection_test_support import EFFECTIVE, INSTANCE, NOW, SESSION, TOKEN, Fixture
from replica_common import Invalid, digest


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

    def test_vault_mode_bundle_bytes_match_pre_policy_release(self):
        # main 959d473（策略模式之前）同一夹具的 bundle_id；它覆盖 manifest 与全部渲染文件的字节。
        for client, expected in (("claude", "ed1de36da342b900a9ec4413873ac84c75c56ab037b63da6dbbe610002c4524d"),
                                 ("codex", "a4f26369ae43fb7b8bcdca5c1c2aa7aa670e44d3830d137359832dc5ac1c6b5c")):
            with self.subTest(client=client):
                self.fixture = Fixture()
                self.fixture.config["client"] = client
                snapshot = self.fixture.fetch()
                bundle = make_bundle(self.fixture.config, snapshot, render(self.fixture.config, snapshot))
                self.assertEqual(bundle["manifest"]["bundle_id"], expected)

    def test_policy_mode_exports_effective_items_as_raw_bytes(self):
        snapshot = self.fixture.use_policy(skills=("demo", "other")).fetch()
        urls = [urllib.parse.urlsplit(call.full_url) for call in self.fixture.calls]
        self.assertEqual([u.path.rsplit("/", 1)[1] for u in urls], ["handshake"] + ["policy"] * 5 + ["handshake"])
        self.assertEqual([urllib.parse.parse_qs(u.query) for u in urls[1:4]],
                         [{"kind": ["rules"]}, {"kind": ["rules"], "subject": ["通用工作规则.md"]}, {"kind": ["skills"]}])
        rule = self.fixture.rules["通用工作规则.md"][1]
        demo = "摘要：演示技能\n触发：新功能 | 重构\n\n# 演示\n\t保留\"原样\"\\\r\n"
        self.assertEqual((snapshot["rules_source"], snapshot["policy_effective"]), ("policy", EFFECTIVE))
        self.assertEqual(snapshot["sources"], [
            {"path": "policy/rules/通用工作规则.md", "body": rule, "policy_version": 3,
             "policy_sha12": digest(rule.encode())[:12]},
            {"path": "policy/skills/demo/SKILL.md", "body": demo, "policy_version": 4,
             "policy_sha12": digest(self.fixture.skill_json("demo").encode())[:12]},
            {"path": "policy/skills/other/SKILL.md", "body": "摘要：其他\n触发：\n\nx\n", "policy_version": 6,
             "policy_sha12": digest(self.fixture.skill_json("other").encode())[:12]}])
        bundle = make_bundle(self.fixture.config, snapshot, render(self.fixture.config, snapshot))
        self.assertEqual(bundle["files"]["sources/policy/skills/demo/SKILL.md"], demo.encode())
        self.assertEqual(bundle["manifest"]["policy_effective"], EFFECTIVE)
        context = bundle["files"]["CONTEXT.md"].decode()
        self.assertIn(f"策略版本 `{EFFECTIVE}`", context)
        self.assertNotIn("a" * 32, context)
        self.assertNotIn(TOKEN, json.dumps(snapshot))
        self.assertNotIn(SESSION, json.dumps(snapshot))
        self.fixture = Fixture()
        self.fixture.use_policy(skills=()).fetch()
        self.assertEqual(len(self.fixture.calls), 4, "只选规则时不读 skills 清单")

    def test_fixture_skill_json_matches_server_serde_bytes(self):
        # 真实 studiolo-agent（main 959d473）bootstrap 后 `/read/policy` 回报的 sha256（serde 规范 JSON），
        # 覆盖引号、反斜杠、制表、CRLF、控制符、DEL、emoji 与 U+2028：夹具合成的服务端正文须与之逐字节一致。
        self.fixture.skills["demo"] = (4, "演示\"引号\\反斜杠\t制表", ["新功能", "写 React 时"],
                                       "# 演示\n\t制表\r\nCRLF\u0001控制\u007f删除 \U0001f600 \u2028段落 </script> \\ \"\n")
        self.assertEqual(digest(self.fixture.skill_json("demo").encode()),
                         "12ae2c45a370f61ba8935874e49519b7d374f22be4e51bf5fbdcee28d472fe62")
        snapshot = self.fixture.use_policy().fetch()
        self.assertEqual(snapshot["sources"][1]["body"],
                         "摘要：演示\"引号\\反斜杠\t制表\n触发：新功能 | 写 React 时\n\n"
                         + self.fixture.skills["demo"][3])

    def test_skill_with_newline_summary_and_pipe_trigger_passes_whole_batch(self):
        # 规范 JSON 按收到的字节核对，摘要含换行、触发含 ` | ` 也能整批通过；旧的文本还原在此必拒。
        self.fixture.skills["demo"] = (4, "演示\n技能", ["新功能 | 重构", "拆分"], "# 演示\n正文\n")
        snapshot = self.fixture.use_policy().fetch()
        demo = [s for s in snapshot["sources"] if s["path"] == "policy/skills/demo/SKILL.md"]
        self.assertEqual(demo, [{"path": "policy/skills/demo/SKILL.md",
                                 "body": "摘要：演示\n技能\n触发：新功能 | 重构 | 拆分\n\n# 演示\n正文\n",
                                 "policy_version": 4,
                                 "policy_sha12": digest(self.fixture.skill_json("demo").encode())[:12]}])
        listing = self.fixture.policy_text({"kind": ["skills"]})
        self.assertIn("demo\tv4\t演示 技能\t新功能 | 重构 | 拆分", listing.split("\n"))

    def test_policy_listing_parses_empty_and_tabbed_rows(self):
        self.assertEqual(fetcher._policy_listing(f"# 策略版本 {EFFECTIVE}\n0 份规则母本\n", "rules"), (EFFECTIVE, {}))
        text = f"# 策略版本 {EFFECTIVE}\n1 个 skill（索引式候选；硬绑定见路牌第 5 项）\ndemo\tv4\t含\t制表\t触发"
        self.assertEqual(fetcher._policy_listing(text, "skills"), (EFFECTIVE, {"demo": (4, None)}))

    def test_policy_version_or_item_mismatch_rejects_whole_batch(self):
        head, moved = f"# 策略版本 {EFFECTIVE}\n", "# 策略版本 fedcba987654\n"
        sha12 = digest(self.fixture.rules["通用工作规则.md"][1].encode())[:12]
        for old, new in ((head + "policy://rules/", moved + "policy://rules/"),
                         (head + "2 个 skill", moved + "2 个 skill"),
                         (head + "policy://skills/", moved + "policy://skills/"),
                         ("通用工作规则.md@v3", "通用工作规则.md@v9"), ("demo@v4", "demo@v9"),
                         ("禁止绕过式修复。", "已被替换。"), ("\\t保留", "\\t替换"),
                         ("\tv3\t" + sha12, "\tv3\t" + "0" * 12),
                         ("policy://rules/通用工作规则.md@", "policy://rules/其他.md@"),
                         ("2 份规则母本", "3 份规则母本"), (head + "2 份", head + "\n2 份")):
            with self.subTest(new=new):
                self.fixture = Fixture().use_policy()
                self.fixture.mutate = lambda route, text, h: text.replace(old, new) if route == "policy" else text
                self.rejected()

    def test_policy_unlisted_name_or_leaked_secret_rejects_batch(self):
        for rules, skills in ((("缺失.md",), ("demo",)), (("通用工作规则.md",), ("missing",))):
            with self.subTest(rules=rules, skills=skills):
                self.fixture = Fixture().use_policy(rules, skills)
                self.rejected()
                subjects = {urllib.parse.parse_qs(urllib.parse.urlsplit(c.full_url).query).get("subject", [""])[0]
                            for c in self.fixture.calls}
                self.assertFalse({"缺失.md", "missing"} & subjects, "不在清单的名字不得发出正文请求")
        for name, value in (("rules", (3, "正文" + TOKEN)), ("skills", (4, "演示", [], "正文" + SESSION)),
                            ("skills", (4, "演示", [TOKEN], "# 演示\n"))):
            with self.subTest(kind=name, value=value):
                self.fixture = Fixture().use_policy()
                getattr(self.fixture, name)["通用工作规则.md" if name == "rules" else "demo"] = value
                self.rejected()
        # 经 JSON 转义的令牌在收到的字节里看不出来，渲染成落盘文本后那一次检查必须拦下。
        self.fixture = Fixture().use_policy()
        self.fixture.raw_skills["demo"] = ('{"name":"demo","summary":"演示","triggers":[],"markdown":"\\u'
                                           + format(ord(TOKEN[0]), "04x") + TOKEN[1:] + '"}')
        self.assertNotIn(TOKEN, self.fixture.raw_skills["demo"])
        self.rejected()

    def test_skill_body_must_be_the_canonical_json_of_that_skill(self):
        # 摘要行与正文字节一致（无撕裂），但正文不是该 skill 的规范 JSON 形状：整批拒绝。
        fields = {"name": "demo", "summary": "演示技能", "triggers": ["新功能"], "markdown": "# 演示\n"}
        for label, raw in (("非 JSON", "摘要：演示技能\n触发：新功能\n\n# 演示\n"),
                           ("名字不符", dict(fields, name="other")),
                           ("多余键", dict(fields, extra="x")),
                           ("缺键", {k: v for k, v in fields.items() if k != "triggers"}),
                           ("触发不是列表", dict(fields, triggers="新功能")),
                           ("触发含非字符串", dict(fields, triggers=["新功能", 1])),
                           ("摘要不是字符串", dict(fields, summary=None)),
                           ("深层嵌套", "[" * 100_000 + "]" * 100_000)):
            with self.subTest(label=label):
                self.fixture = Fixture().use_policy()
                self.fixture.raw_skills["demo"] = raw if isinstance(raw, str) else json.dumps(
                    raw, ensure_ascii=False, separators=(",", ":"))
                self.rejected()


if __name__ == "__main__":
    unittest.main()
