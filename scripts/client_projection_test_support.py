"""投影契约测试使用的虚构资料与内存 HTTP 供给，不读取真实凭据或Vault。"""
import importlib.util
import io
import json
from pathlib import Path
import time
import urllib.error
import urllib.parse
from email.message import Message
from unittest.mock import patch

import client_projection_fetch as fetcher
from client_projection_render import render
from client_projection_schema import NATIVE, make_bundle
from replica_common import digest

NOW = 1_789_056_579_000
INSTANCE = "01K00000000000000000000000"
SESSION = "01K11111111111111111111111"
TOKEN = "PUBLIC_TEST_ONLY_CANARY"
EFFECTIVE = "0123456789ab"
POLICY_SKILL = "摘要：演示\n触发：新功能\n\n# 演示\n正文。\n"


def fixture_config(client="claude", url="http://127.0.0.1:4400/read"):
    return {"schema_version": 1, "client": client, "project": "项目/Studiolo",
            "source_url": url, "token_env": "STUDIOLO_FIXTURE_TOKEN",
            "rules": ["_规则/rules.md"], "skills": []}


def fixture_bundle(body="# 规则\n测试正文。\n", client="claude", instance="0" * 26):
    cfg = fixture_config(client)
    now = int(time.time() * 1000)
    snapshot = {"identity": {"type": "fixture", "instance": instance},
                "freshness": {"generation": "a" * 32, "source_digest": "b" * 64,
                              "source_observed_at_ms": now - 1000, "received_at_ms": now - 500},
                "generated_at_ms": now,
                "sources": [{"path": "_规则/rules.md", "revision": "c" * 64, "body": body}]}
    files = {p: b"fixture\n" for p in NATIVE[client] | {"CONTEXT.md"}}
    files["sources/_规则/rules.md"] = body.encode()
    return make_bundle(cfg, snapshot, files)


def fixture_policy_config(client="claude", url="http://127.0.0.1:4400/read"):
    return dict(fixture_config(client, url), rules_source="policy",
                rules=["通用工作规则.md"], skills=["demo"])


def fixture_policy_snapshot(body="# 规则\n测试正文。\n", effective=EFFECTIVE,
                            generation="a" * 32, instance="0" * 26, skill=POLICY_SKILL):
    """策略模式快照：规则短摘要即正文摘要前 12 位；skill 短摘要本应取自服务端规范 JSON，schema 不复算，这里借正文摘要充当。"""
    now = int(time.time() * 1000)
    return {"identity": {"type": "fixture", "instance": instance},
            "freshness": {"generation": generation, "source_digest": "b" * 64,
                          "source_observed_at_ms": now - 1000, "received_at_ms": now - 500},
            "generated_at_ms": now, "rules_source": "policy", "policy_effective": effective,
            "sources": [{"path": "policy/rules/通用工作规则.md", "body": body, "policy_version": 3,
                         "policy_sha12": digest(body.encode())[:12]},
                        {"path": "policy/skills/demo/SKILL.md", "body": skill, "policy_version": 4,
                         "policy_sha12": digest(skill.encode())[:12]}]}


def fixture_policy_bundle(client="claude", **snapshot):
    cfg, snap = fixture_policy_config(client), fixture_policy_snapshot(**snapshot)
    files = {p: b"fixture\n" for p in NATIVE[client] | {"CONTEXT.md"}}
    files.update({"sources/" + s["path"]: s["body"].encode() for s in snap["sources"]})
    return make_bundle(cfg, snap, files)


def fixture_policy_projection(client="claude", **snapshot):
    """与 fixture_policy_bundle 同一快照，但客户端文件由真实 render 生成。"""
    cfg, snap = fixture_policy_config(client), fixture_policy_snapshot(**snapshot)
    return make_bundle(cfg, snap, render(cfg, snap))


def load_sync_cli():
    """按文件加载带连字符的 CLI 模块，供契约测试直接调用其 run()。"""
    spec = importlib.util.spec_from_file_location(
        "sync_agent_client", Path(__file__).with_name("sync-agent-client.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class Response(io.BytesIO):
    def __init__(self, target, body, headers):
        super().__init__(body.encode("utf-8") if isinstance(body, str) else body)
        self.status, self.target, self.headers = 200, target, headers

    def geturl(self):
        return self.target


class Fixture:
    """内存 HTTP 供给：vault 路由是原抓取夹具；策略路由逐字对照 server.rs `text_policy_read` 合成。"""

    def __init__(self):
        self.config = {"schema_version": 1, "client": "claude", "project": "项目/Alpha",
                       "source_url": "https://example.invalid/read", "token_env": "TEST_TOKEN",
                       "rules": ["_规则/准则.md", "项目/Alpha/CLAUDE.md"],
                       "skills": ["_规则/skills/explain"]}
        self.notes = {"_规则/准则.md": "规则正文\n", "项目/Alpha/CLAUDE.md": "项目规则\n",
                      "_规则/skills/explain/SKILL.md": "---\nname: explain\n---\n参考 [详情](resources/详情.md)\n",
                      "_规则/skills/explain/resources/详情.md": "﻿原文\r\n保留字节\r\n"}
        self.extra, self.calls, self.mutate = [], [], lambda route, text, headers: text
        self.blank_line = True
        self.effective = EFFECTIVE
        self.rules = {"通用工作规则.md": (3, "# 规则\n禁止绕过式修复。\n"), "其他.md": (5, "其他\n")}
        self.skills = {"demo": (4, "演示技能", ["新功能", "重构"], "# 演示\n\t保留\"原样\"\\\r\n"),
                       "other": (6, "其他", [], "x\n")}

    def use_policy(self, rules=("通用工作规则.md",), skills=("demo",)):
        self.config.update(rules_source="policy", rules=list(rules), skills=list(skills))
        return self

    def fresh(self):
        return ("副本状态 ready；stale=false；generation=" + "a" * 32 + "；source_digest=" + "b" * 64
                + f"；source_observed_at_ms={NOW - 1000}；received_at_ms={NOW - 500}；age_secs=1；max_age_secs=300")

    def skill_json(self, name):
        version, summary, triggers, markdown = self.skills[name]
        return json.dumps({"name": name, "summary": summary, "triggers": triggers, "markdown": markdown},
                          ensure_ascii=False, separators=(",", ":"))

    def policy_text(self, query):
        """未知名字返回 None，由调用方按服务端 404 处理。"""
        kind, subject, head = query["kind"][0], query.get("subject", [None])[0], f"# 策略版本 {self.effective}\n"
        if kind == "rules" and subject is None:
            rows = [f"{name}\tv{v}\t{digest(body.encode())[:12]}" for name, (v, body) in self.rules.items()]
            return head + f"{len(rows)} 份规则母本\n" + "\n".join(rows)
        if subject is None:
            rows = [f"{name}\tv{v}\t{s}\t{' | '.join(t)}" for name, (v, s, t, _) in self.skills.items()]
            return head + f"{len(rows)} 个 skill（索引式候选；硬绑定见路牌第 5 项）\n" + "\n".join(rows)
        if subject not in (self.rules if kind == "rules" else self.skills):
            return None
        if kind == "rules":
            version, body = self.rules[subject]
            return head + f"policy://rules/{subject}@v{version} sha256 {digest(body.encode())}\n{body}"
        version, summary, triggers, markdown = self.skills[subject]
        return (head + f"policy://skills/{subject}@v{version} sha256 {digest(self.skill_json(subject).encode())}\n"
                f"摘要：{summary}\n触发：{' | '.join(triggers)}\n\n{markdown}")

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
        elif route == "policy":
            text = self.policy_text(query)
            if text is None:
                raise urllib.error.HTTPError(request.full_url, 404, "not found", headers, io.BytesIO())
        else:
            body = self.notes[query["path"][0]]
            # 故意使用不同于正文 SHA 的 revision，匹配实际 Vault 域分离协议。
            text = "revision " + "c" * 64 + "\n" + self.fresh() + "\n" + body
        return Response(request.full_url, self.mutate(route, text, headers), headers)

    def fetch(self):
        with patch.object(fetcher.urllib.request, "build_opener", return_value=self), \
                patch.object(fetcher.time, "time", return_value=NOW / 1000):
            return fetcher.fetch(self.config, TOKEN)
