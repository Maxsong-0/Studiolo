#!/usr/bin/env python3
"""虚构托管副本 → 真实 HTTP/审计 → 客户端投影的端到端验收。"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
from unittest.mock import patch

sys.dont_write_bytecode = True

from client_projection_fetch import Client, fetch
from client_projection_io import read_bundle
from client_projection_render import render
from client_projection_schema import config, make_bundle
from client_projection_store import apply, check, plan
from client_projection_test_support import fixture_config
from replica_common import Invalid

SCRIPTS = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("replica_pipeline", SCRIPTS / "check-replica-pipeline.py")
PIPE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PIPE)


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def denied(operation):
    try:
        operation()
    except Invalid:
        return
    raise RuntimeError("应当拒绝的导出仍返回资料")


def cli(*args, token, expected=0):
    env = dict(os.environ, STUDIOLO_FIXTURE_TOKEN=token)
    result = subprocess.run([sys.executable, "-B", str(SCRIPTS / "sync-agent-client.py"), *map(str, args)],
                            env=env, capture_output=True, text=True, timeout=60)
    require(token not in result.stdout + result.stderr, "CLI泄露秘密凭据")
    require(result.returncode == expected, f"CLI {args[0]} 返回码 {result.returncode}，预期 {expected}")
    return json.loads(result.stdout)


def export(cfg, token):
    snap = fetch(config(cfg), token)
    return make_bundle(cfg, snap, render(cfg, snap))


def inspect_audit(pipeline, manifest):
    with sqlite3.connect(pipeline.data / "agent.sqlite3") as db:
        rows = db.execute("SELECT type_name,instance_id,path,revision,tool,ledger_offset "
                          "FROM reads WHERE instance_id=?", (manifest["identity"]["instance"],)).fetchall()
    for source in manifest["sources"]:
        require(any(row[:5] == (manifest["identity"]["type"], manifest["identity"]["instance"],
                               source["path"], source["revision"], "read_note") for row in rows),
                "源笔记缺少相同身份与revision的审计读取索引")
    require(all(row[5] >= 0 for row in rows), "读取未关联权威账本偏移")
    with sqlite3.connect(pipeline.data / "audit/audit.sqlite3") as ledger:
        offsets = {row[0] for row in ledger.execute("SELECT offset FROM audit_events")}
    require(all(row[5] in offsets for row in rows), "派生索引引用不存在的权威账本事件")


def run(binary):
    with tempfile.TemporaryDirectory(prefix="studiolo-client-pipeline-") as tmp:
        root = Path(tmp).resolve()
        p = PIPE.Pipeline(binary, root)
        try:
            PIPE.write(p.source / "项目/Studiolo/CLAUDE.md", "# 项目规则\n只读参考。\n")
            PIPE.write(p.source / "项目/Studiolo/04-协调/TASKS.md", "# BLACKBOARD_PRIVATE_STATE\n")
            PIPE.write(p.source / "_规则/skills/demo/SKILL.md",
                       "---\nname: demo\ndescription: 虚构参考\n---\n读取 references/detail.md。\n")
            PIPE.write(p.source / "_规则/skills/demo/references/detail.md", "# 技能细则\nFIRST_RULE\n")
            PIPE.write(p.source / "_规则/skills/demo/references/index.md", "# 静态技能目录\n")
            p.push()
            p.start()
            # 发布探针在真实隔离 HTTP 服务上验收，避免只验证自己构造的协议样例。
            token_path = root / "http-probe.token"
            PIPE.write(token_path, p.token)
            probe = subprocess.run([sys.executable, "-B", str(SCRIPTS / "check-agent-http.py"),
                                    p.url.removesuffix("/mcp"), str(token_path)],
                                   capture_output=True, text=True, timeout=60)
            require(p.token not in probe.stdout + probe.stderr, "HTTP探针泄露凭据")
            require(probe.returncode == 0, "真实 HTTP 发布探针失败：" + probe.stderr.strip())
            token_path.unlink()
            cfg = fixture_config(url=p.url.replace("/mcp", "/read"))
            cfg["rules"].append("项目/Studiolo/CLAUDE.md")
            cfg["skills"] = ["_规则/skills/demo"]
            cfg_path, out, target = (root / name for name in ("config.json", "bundle", "client"))
            PIPE.write(cfg_path, json.dumps(cfg, ensure_ascii=False))
            cli("generate", "--config", cfg_path, "--out", out, token=p.token)
            bundle = read_bundle(out)
            require(bundle["manifest"]["identity"]["type"] == "fixture", "导出身份被客户端格式替换")
            inspect_audit(p, bundle["manifest"])
            for raw in bundle["files"].values():
                for secret in (p.token, "BLACKBOARD_PRIVATE_STATE", "DO_NOT_SEND", "FIRST_CONTENT"):
                    require(secret.encode() not in raw, "投影包含未选资料、协作状态或凭据")
            require(cli("plan", "--bundle", out, "--target", target, token=p.token)["status"] == "create",
                    "初次预览结果错误")
            require(not target.exists(), "预览改变目标")
            cli("apply", "--bundle", out, "--target", target, token=p.token)
            require(cli("check", "--target", target, token=p.token)["source_status"] == "unknown_offline",
                    "离线检查冒充来源新鲜度")
            require(cli("check", "--target", target, "--online", token=p.token)["source_status"] == "unchanged",
                    "在线来源未正确复验")
            again = export(cfg, p.token)
            require(plan(target, again)["status"] == "unchanged", "重复导出未幂等")
            codex_cfg = dict(cfg, client="codex")
            codex = export(codex_cfg, p.token)
            apply(root / "codex", codex)
            require(".codex/config.toml" in codex["files"], "Codex配置未生成")
            print("通过：两类客户端、五份源文、真实审计、CLI预览/应用/离线检查与幂等")

            PIPE.write(p.source / "_规则/rules.md", "# 更新规则\nSECOND_RULE\n")
            p.push()
            require(cli("check", "--target", target, "--online", token=p.token, expected=2)["source_status"]
                    == "source_changed", "来源更新未检测")
            updated = export(cfg, p.token)
            apply(target, updated)
            previous = os.readlink(target / "current")
            current_file = target / previous / "CLAUDE.md"
            before = current_file.read_bytes()
            current_file.write_text("用户编辑，必须保留", encoding="utf-8")
            require(check(target)["status"] == "local_modified", "未检测用户修改")
            require(plan(target, bundle)["status"] == "conflict", "覆盖用户修改未拦截")
            denied(lambda: apply(target, bundle))
            require(cli("apply", "--bundle", out, "--target", target, token=p.token, expected=1)["status"]
                    == "failed", "apply重验失败语义错误")
            require(current_file.read_text() == "用户编辑，必须保留", "用户资料被覆盖")
            current_file.write_bytes(before)
            print("通过：来源漂移、完整更新、原批保留及本地手改拒绝")

            original_get, switched = Client.get, []

            def switch_after_first_note(client, name, arguments=None):
                reply = original_get(client, name, arguments)
                if name == "note" and not switched:
                    switched.append(True)
                    PIPE.write(p.source / "_规则/rules.md", "# 第三批\nTHIRD_RULE\n")
                    p.push()
                return reply

            with patch.object(Client, "get", switch_after_first_note):
                denied(lambda: export(cfg, p.token))
            require(os.readlink(target / "current") == previous, "混批失败改变了已安装副本")
            denied(lambda: export(cfg, "invalid-fixture-token"))
            missing = dict(cfg, rules=["_规则/missing.md"])
            denied(lambda: export(missing, p.token))
            forbidden = dict(cfg, rules=["School/private.md"])
            denied(lambda: export(forbidden, p.token))
            print("通过：抓取中途换代整体拒绝、错误凭据、缺失源与越界源拒绝")

            PIPE.write(p.source / "_规则/skills/demo/script.py", "print('must not export')\n")
            p.push()
            denied(lambda: export(cfg, p.token))
            (p.source / "_规则/skills/demo/script.py").unlink()
            p.push()
            (p.data / "audit").chmod(0o750)
            denied(lambda: export(cfg, p.token))
            (p.data / "audit").chmod(0o700)
            export(cfg, p.token)
            p.admin("type-revoke", "--type", "fixture")
            denied(lambda: export(cfg, p.token))
            require(cli("check", "--target", target, "--online", token=p.token, expected=3)["source_status"]
                    == "unavailable", "吊销后来源被标为已确认")
            p.stop()
            require(cli("check", "--target", target, token=p.token)["status"] == "ok", "离线已验证副本不可读")
            print("通过：非Markdown技能拒绝、审计故障、恢复、吊销拒绝及离线保留")
        finally:
            p.stop()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary")
    args = parser.parse_args()
    try:
        run(str(Path(args.binary).resolve(strict=True)))
    except Exception as error:
        print("客户端投影管线失败：" + (str(error) if isinstance(error, RuntimeError)
                                  else type(error).__name__), file=sys.stderr)
        sys.exit(1)
