"""投影契约测试使用的虚构资料，不读取真实凭据或Vault。"""
import time

from client_projection_schema import NATIVE, make_bundle


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
