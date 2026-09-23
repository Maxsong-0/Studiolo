"""策略模式（rules_source: "policy"）的真实 agent 端到端验收：隔离策略平面 → GET /read/policy → 客户端投影。

由 check-client-projection-pipeline.py 在 vault 流程之后调用。真实 studiolo-agent 在真实 PTY 下
policy-bootstrap 能力集、规则母本与含特殊字符的 skill，再带 --policy 启动；夹具全部虚构，
不读真实 Vault、策略库或凭据。
"""
from contextlib import closing
import json
import os
from pathlib import Path
import re
import sqlite3
import tempfile

from client_projection_io import read_bundle
from client_projection_test_support import fixture_config
from replica_common import digest

RULE, LATER_RULE, SKILL_NAME = "通用工作规则.md", "新增规则.md", "demo-skill"
# 规则母本入库只把 CRLF 统一为 LF；正文只用 LF，落盘字节才能与发布正文逐字节相等。
RULE_BODY = "# 通用工作规则\n\n## 准则① 永远说中文\n\n禁止绕过式修复；改码前先读协调状态。\n"
LATER_BODY = "# 新增规则\n\n首轮导出之后才发布，且不在投影所选之列。\n"
# 摘要含全角标点、emoji、ASCII 引号与反斜杠；摘要与触发都不含换行，触发不含 ` | `（文本协议无法唯一还原）。
SKILL = {"name": SKILL_NAME,
         "summary": "演示技能：全角「引号」（括号）！emoji 🚀✨，ASCII \"quote\" 与反斜杠 \\",
         "triggers": ["写或改 React / Next.js 代码", "重构：拆分组件？"],
         "markdown": "# 演示技能\n\n第一段：先读规则，再动手。\n\n```python\ndef greet():\n"
                     "\tprint(\"你好，\\\"世界\\\" 🚀\")\n```\n\n末段：结束。\n"}
SKILL_TEXT = f"摘要：{SKILL['summary']}\n触发：{' | '.join(SKILL['triggers'])}\n\n{SKILL['markdown']}"
# 类型名须与 Pipeline 签发的类型一致；只读投影不需要任何写或提案能力。
CAPABILITY = {"ai_type": "fixture",
              "read": {"project_whitelist": ["项目/Studiolo"], "deny_by_default": True},
              "write": {"board": False, "content_patch_only": False, "content_direct": False},
              "policy": {"propose_desired": False, "activate": False}, "hard_bound_skills": []}
# 格式唯一来源：crates/agent-server/src/admin/policy.rs `bootstrap` 的控制终端回显。
BOOTSTRAPPED = re.compile(r"已 bootstrap：([a-z_]+) (.+?) → 版本 ([1-9][0-9]*)，effective ([a-f0-9]{64})")


def bootstrap(pipe, p, policy, kind, subject, body):
    """真实 PTY 下 policy-bootstrap 一份首版，返回 (版本, effective)；只能写从未有过版本的 subject。"""
    source = p.root / "policy-input" / f"{kind}-{digest(subject.encode('utf-8'))[:12]}"
    pipe.write(source, body)
    code, output = p.admin_tty("policy-bootstrap", "--policy", str(policy), "--kind", kind,
                               "--subject", subject, "--file", str(source))
    # 终端输出只有加固分类与固定结果/诊断，不含正文；admin_tty 已核验其中没有类型凭据。
    detail = ([line.strip() for line in output.splitlines() if line.strip()] or ["无输出"])[-1]
    pipe.require(code == 0, f"策略 bootstrap {kind} 失败（真实 PTY 下）：{detail}")
    match = BOOTSTRAPPED.search(output)
    pipe.require(match is not None and match.group(1, 2) == (kind, subject),
                 f"控制终端没有确认 bootstrap {kind}")
    return int(match[3]), match[4]


def policy_status(pipe, p, policy):
    """admin policy-status 的 effective 与现行各项 {(种类, subject): (版本, 短摘要)}，供与导出交叉核对。"""
    effective, entries = None, {}
    for fields in (line.split("\t") for line in p.admin("policy-status", "--policy", str(policy),
                                                        data=False).splitlines()):
        if fields[0] == "effective" and len(fields) == 2:
            effective = fields[1]
        elif fields[0] in ("capability_set", "rules", "skills") and len(fields) == 5:
            entries[(fields[0], fields[1])] = (int(fields[2].removeprefix("v")), fields[3])
    pipe.require(effective is not None and re.fullmatch("[a-f0-9]{64}", effective) is not None,
                 "policy-status 未给出 effective")
    return effective, entries


def inspect_policy_audit(pipe, p, manifest, exports):
    """每次导出恰好读两份清单加所选各项正文，全部以 policy_read 入审计并关联权威账本事件。"""
    expected = [("policy://rules", "-"), ("policy://skills", "-")]
    for source in manifest["sources"]:
        _, kind, name = source["path"].removesuffix("/SKILL.md").split("/", 2)
        expected.append((f"policy://{kind}/{name}", f"v{source['policy_version']}"))
    with closing(sqlite3.connect(p.data / "agent.sqlite3")) as db:
        rows = db.execute("SELECT type_name,instance_id,path,revision,tool,ledger_offset FROM reads").fetchall()
    reads = {}
    for kind, instance, path, revision, tool, _ in rows:
        pipe.require(kind == manifest["identity"]["type"] and tool == "policy_read",
                     "策略模式出现了非 policy_read 的读取审计")
        reads.setdefault(instance, []).append((path, revision))
    pipe.require(manifest["identity"]["instance"] in reads and len(reads) == exports,
                 "策略模式审计里的导出实例数与实际导出次数不符")
    pipe.require(all(sorted(seen) == sorted(expected) for seen in reads.values()),
                 "策略模式每次导出的读取审计不是恰好两份清单加所选各项（路径 policy://…、revision v<版本>）")
    with closing(sqlite3.connect(p.data / "audit/audit.sqlite3")) as ledger:
        events = {offset: json.loads(raw)["event"]
                  for offset, raw in ledger.execute("SELECT offset,record_json FROM audit_events")}
    # 派生索引之外再核权威账本本身：索引的每一行都对应同一实例、路径、revision 与工具的 content.read 事件。
    pipe.require(all(events.get(offset, {}).get("action") == "content.read"
                     and all(events[offset]["payload"].get(key) == value for key, value in
                             (("agent_type", kind), ("agent_instance", instance), ("path", path),
                              ("revision", revision), ("tool", tool)))
                     for kind, instance, path, revision, tool, offset in rows),
                 "策略读取索引与权威账本的 content.read 事件不一致")


def run_policy(binary, pipe, cli):
    """策略模式全流程；pipe 是已加载的 check-replica-pipeline 模块，cli 是主管线的 sync-agent-client 调用器。"""
    # 与 vault 流程同一前提：夹具目录权限不随环境 umask 漂移（CI 另在 umask 002 下再跑一遍）。
    os.umask(0o022)
    with tempfile.TemporaryDirectory(prefix="studiolo-client-policy-") as tmp:
        root = Path(tmp).resolve()
        p = pipe.Pipeline(binary, root)
        try:
            policy = root / "policy"
            policy.mkdir(mode=0o700)
            p.admin("policy-init", "--policy", str(policy), data=False)
            versions = {}
            for kind, subject, body in (("capability_set", "fixture", json.dumps(CAPABILITY, ensure_ascii=False)),
                                        ("rules", RULE, RULE_BODY),
                                        ("skills", SKILL_NAME, json.dumps(SKILL, ensure_ascii=False))):
                versions[(kind, subject)], effective = bootstrap(pipe, p, policy, kind, subject, body)
            status_effective, status = policy_status(pipe, p, policy)
            pipe.require(status_effective == effective and set(status) == set(versions)
                         and all(status[key][0] == version for key, version in versions.items())
                         and status[("rules", RULE)][1] == digest(RULE_BODY.encode("utf-8"))[:12],
                         "policy-status 与 bootstrap 回显的版本、摘要或 effective 不一致")
            p.push()
            p.start(policy_dir=policy)
            short = effective[:12]
            cfg = dict(fixture_config(url=p.url.replace("/mcp", "/read")), rules_source="policy",
                       rules=[RULE], skills=[SKILL_NAME])
            cfg_path, out, target = (root / name for name in ("config.json", "bundle", "client"))
            pipe.write(cfg_path, json.dumps(cfg, ensure_ascii=False))
            generated = cli("generate", "--config", cfg_path, "--out", out, token=p.token)
            pipe.require(generated["policy_effective"] == short and generated["sources"] == 2,
                         "策略模式导出未报出现行策略版本或源文数不符")
            bundle = read_bundle(out)
            manifest = bundle["manifest"]
            pipe.require(manifest["rules_source"] == "policy" and manifest["policy_effective"] == short,
                         "manifest 未记录策略来源与 12 位策略版本")
            pipe.require({s["path"]: (s["policy_version"], s["policy_sha12"]) for s in manifest["sources"]}
                         == {"policy/rules/" + RULE: status[("rules", RULE)],
                             "policy/skills/" + SKILL_NAME + "/SKILL.md": status[("skills", SKILL_NAME)]},
                         "manifest 逐项策略版本或短摘要与策略平面不一致")
            for raw in bundle["files"].values():
                pipe.require(p.token.encode() not in raw, "策略投影包含凭据")
            planned = cli("plan", "--bundle", out, "--target", target, token=p.token)
            pipe.require(planned["status"] == "create" and planned["policy"]["new_effective"] == short,
                         "策略模式初次预览结果错误")
            pipe.require(not target.exists(), "预览改变目标")
            cli("apply", "--bundle", out, "--target", target, token=p.token)
            installed = target / "current" / "sources" / "policy"
            pipe.require((installed / "rules" / RULE).read_bytes() == RULE_BODY.encode("utf-8"),
                         "规则母本落盘字节与发布正文不同")
            pipe.require((installed / "skills" / SKILL_NAME / "SKILL.md").read_bytes() == SKILL_TEXT.encode("utf-8"),
                         "skill 落盘不是「摘要、触发、空行、正文」原样")
            offline = cli("check", "--target", target, token=p.token)
            pipe.require(offline["source_status"] == "unknown_offline" and offline["policy_effective"] == short,
                         "策略模式离线检查冒充来源新鲜度")
            online = cli("check", "--target", target, "--online", token=p.token)
            pipe.require(online["source_status"] == "unchanged"
                         and online["policy"] == {"old_effective": short, "new_effective": short, "items": []},
                         "策略模式在线来源未正确复验")
            print("通过：策略模式真实 PTY bootstrap → /read/policy 导出、预览、安装、离线/在线检查，落盘逐字节一致")

            _, later = bootstrap(pipe, p, policy, "rules", LATER_RULE, LATER_BODY)
            pipe.require(later[:12] != short, "新增规则未产生新的策略版本")
            drift = cli("check", "--target", target, "--online", token=p.token, expected=2)
            pipe.require(drift["source_status"] == "source_changed", "新增规则后在线检查未报 source_changed")
            # 未选中的条目变化同样改变策略版本；逐项明细为空，表示所选资料本身未变。
            pipe.require(drift["policy"] == {"old_effective": short, "new_effective": later[:12], "items": []},
                         "策略漂移明细应只有策略版本变化")
            inspect_policy_audit(pipe, p, manifest, exports=3)
            print("通过：策略模式新增未选规则后 check --online 报 source_changed（退出码 2），三次导出读取全部 policy_read 入审计")
        finally:
            p.stop()
