"""客户端投影的闭集配置、来源清单及可重算完整性声明。"""
import re
from urllib.parse import urlsplit

from replica_common import digest, encode, integer, logical_path_ok, require

MAX_SOURCE = 4 * 1024 * 1024
MAX_TOTAL = 16 * 1024 * 1024
MAX_SOURCES = 128
MAX_MANIFEST = 256 * 1024
NATIVE = {
    "claude": {"CLAUDE.md", ".mcp.json", ".claude/skills/studiolo-context/SKILL.md"},
    "codex": {"AGENTS.md", ".codex/config.toml", ".agents/skills/studiolo-context/SKILL.md"},
}
CONFIG_KEYS = {"schema_version", "client", "project", "source_url", "token_env", "rules", "skills"}
MANIFEST_KEYS = {"schema_version", "bundle_id", "projection_key", "config", "identity",
                 "freshness", "generated_at_ms", "sources", "files"}


def sha(value):
    require(isinstance(value, str) and re.fullmatch("[a-f0-9]{64}", value))


def static_source(path):
    parts = logical_path_ok(path)
    forbidden = {"02-日志", "04-协调"}
    if not path.startswith("_规则/skills/"):
        forbidden |= {"tasks.md", "index.md"}
    require(not any(p.casefold() in forbidden for p in parts))


def config(value):
    require(isinstance(value, dict) and set(value) == CONFIG_KEYS)
    require(type(value["schema_version"]) is int and value["schema_version"] == 1)
    require(value["client"] in NATIVE)
    project = logical_path_ok(value["project"])
    require(len(project) == 2 and project[0] == "项目")
    endpoint = value["source_url"]
    require(isinstance(endpoint, str) and len(endpoint) <= 2048 and endpoint.isascii())
    require(not any(c.isspace() or ord(c) < 32 or ord(c) == 127 for c in endpoint))
    url = urlsplit(endpoint)
    require(url.scheme in ("http", "https") and url.hostname)
    require(url.path == "/read" and not url.query and not url.fragment)
    require(url.username is None and url.password is None)
    require(url.port is None or 1 <= url.port <= 65535)
    require(url.hostname == "::1" or re.fullmatch(r"[A-Za-z0-9.-]+", url.hostname))
    require(url.scheme == "https" or url.hostname in ("127.0.0.1", "::1"))
    require(isinstance(value["token_env"], str) and
            re.fullmatch("[A-Z][A-Z0-9_]{0,127}", value["token_env"]))
    rules, skills = value["rules"], value["skills"]
    require(isinstance(rules, list) and isinstance(skills, list))
    require(1 <= len(rules) <= 64 and len(skills) <= 32)
    selected = []
    for path in rules:
        parts = logical_path_ok(path)
        require(path.endswith(".md") and (parts[0] == "_规则" or
                path == value["project"] + "/CLAUDE.md"))
        static_source(path)
        selected.append(path.casefold())
    for path in skills:
        parts = logical_path_ok(path)
        require(len(parts) == 3 and parts[:2] == ["_规则", "skills"])
        selected.append(path.casefold())
    require(len(set(selected)) == len(selected))
    require(not any(rule.startswith(skill + "/") for rule in rules for skill in skills))
    return value


def source_allowed(path, cfg):
    logical_path_ok(path)
    require(path.endswith(".md"))
    require(path in cfg["rules"] or any(path.startswith(s + "/") for s in cfg["skills"]))
    static_source(path)


def table(items, keys):
    require(isinstance(items, list) and 0 < len(items) <= MAX_SOURCES + 4)
    previous, folded = "", set()
    for item in items:
        require(isinstance(item, dict) and set(item) == keys)
        path = item["path"]
        require(isinstance(path, str) and path > previous and path.casefold() not in folded)
        previous = path
        folded.add(path.casefold())
        require(integer(item["bytes"]) and 0 <= item["bytes"] <= MAX_SOURCE)
        sha(item["sha256"])


def projection_key(manifest):
    """同来源重复导出不因新的审计实例或时间戳触发安装。"""
    stable = {key: manifest[key] for key in ("schema_version", "config", "sources", "files")}
    stable["type"] = manifest["identity"]["type"]
    stable["freshness"] = {k: manifest["freshness"][k] for k in ("generation", "source_digest")}
    return digest(encode(stable))


def validate_manifest(value):
    require(isinstance(value, dict) and set(value) == MANIFEST_KEYS)
    require(type(value["schema_version"]) is int and value["schema_version"] == 1)
    require(len(encode(value)) <= MAX_MANIFEST)
    cfg = config(value["config"])
    ident = value["identity"]
    require(isinstance(ident, dict) and set(ident) == {"type", "instance"})
    require(isinstance(ident["type"], str) and re.fullmatch("[a-z0-9][a-z0-9-]{0,31}", ident["type"]))
    require(isinstance(ident["instance"], str) and
            re.fullmatch("[0-7][0-9A-HJKMNP-TV-Z]{25}", ident["instance"]))
    fresh = value["freshness"]
    require(isinstance(fresh, dict) and set(fresh) == {
        "generation", "source_digest", "source_observed_at_ms", "received_at_ms"})
    require(isinstance(fresh["generation"], str) and re.fullmatch("[a-f0-9]{32}", fresh["generation"]))
    sha(fresh["source_digest"])
    observed, received, generated = (fresh["source_observed_at_ms"], fresh["received_at_ms"],
                                      value["generated_at_ms"])
    require(all(integer(n) and 0 < n < 10**16 for n in (observed, received, generated)))
    require(observed <= received and observed <= generated + 60_000)
    sources = value["sources"]
    table(sources, {"path", "revision", "bytes", "sha256"})
    require(len(sources) <= MAX_SOURCES and sum(s["bytes"] for s in sources) <= MAX_TOTAL)
    for item in sources:
        source_allowed(item["path"], cfg)
        sha(item["revision"])
    paths = {s["path"] for s in sources}
    require(set(cfg["rules"]).issubset(paths))
    require(all(s + "/SKILL.md" in paths for s in cfg["skills"]))
    files = value["files"]
    table(files, {"path", "bytes", "sha256"})
    expected = NATIVE[cfg["client"]] | {"CONTEXT.md"} | {"sources/" + p for p in paths}
    require({f["path"] for f in files} == expected)
    require(len({p.casefold() for p in expected}) == len(expected))
    lookup = {f["path"]: f for f in files}
    for item in sources:
        out = lookup["sources/" + item["path"]]
        require(out["sha256"] == item["sha256"] and out["bytes"] == item["bytes"])
    require(all(f["bytes"] <= 65536 for f in files if not f["path"].startswith("sources/")))
    sha(value["projection_key"])
    require(value["projection_key"] == projection_key(value))
    sha(value["bundle_id"])
    require(value["bundle_id"] == digest(encode({k: v for k, v in value.items() if k != "bundle_id"})))
    return value


def validate_bundle(bundle):
    require(isinstance(bundle, dict) and set(bundle) == {"manifest", "files"})
    manifest = validate_manifest(bundle["manifest"])
    files = bundle["files"]
    require(isinstance(files, dict) and set(files) == {f["path"] for f in manifest["files"]})
    for item in manifest["files"]:
        raw = files[item["path"]]
        require(isinstance(raw, bytes) and len(raw) == item["bytes"] and digest(raw) == item["sha256"])
        raw.decode("utf-8")
    return bundle


def make_bundle(cfg, snapshot, files):
    manifest = {"schema_version": 1, "config": cfg, **{k: snapshot[k] for k in
                ("identity", "freshness", "generated_at_ms")}}
    manifest["sources"] = [{"path": s["path"], "revision": s["revision"],
                            "bytes": len(s["body"].encode("utf-8")),
                            "sha256": digest(s["body"].encode("utf-8"))}
                           for s in sorted(snapshot["sources"], key=lambda s: s["path"])]
    manifest["files"] = [{"path": path, "bytes": len(raw), "sha256": digest(raw)}
                         for path, raw in sorted(files.items())]
    manifest["projection_key"] = projection_key(manifest)
    manifest["bundle_id"] = digest(encode(manifest))
    return validate_bundle({"manifest": manifest, "files": files})


# 避免存储模块需要了解目录读取实现；延迟导入解决schema与IO的单向校验依赖。
def read_bundle(path):
    from client_projection_io import read_bundle as read
    return read(path)
