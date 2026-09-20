"""私有快照、release、配置及 SQLite 结构契约。"""
import hashlib
import re
import time
from human_snapshot_io import (
    MAX_CIPHER, MAX_FILE, MAX_NODES, MAX_TOTAL, DB_SECONDS,
    absolute, decode, parents, require, safe,
)

SEED_REF = "studiolo-human-totp-v2"
AGE_HEADER = b"age-encryption.org/v1\n"
BASE_FILES = {"auth/auth.sqlite3", "audit/audit.sqlite3", "owner.json", "portable-factor.age"}
BINDING_KEYS = {"auth_directory", "audit_directory", "config", "release", "portable_factor", "seed_ref", "credential_id"}
MANIFEST_KEYS = {"schema_version", "state", "snapshot_id", "source_binding", "source_release_manifest_sha256", "files", "directories"}
RELEASE_REQUIRED = {"bin/studiolo-local-server", "launch-human.py", "studiolo-human.service", "web/index.html"}


def config_binding(raw, binding):
    value = decode(raw)
    required = {"version", "auth_directory", "audit_directory", "owner", "idle_seconds", "absolute_seconds", "seed_ref"}
    optional = {"human_content", "project_repository", "sync_scope", "file_operations"}
    require(type(value) is dict and required <= set(value) <= required | optional)
    require(type(value["version"]) is int and value["version"] == 1)
    require(value["seed_ref"] == SEED_REF)
    require(value["auth_directory"] == binding["auth_directory"] and value["audit_directory"] == binding["audit_directory"])
    require(type(value["owner"]) is dict and set(value["owner"]) == {"issuer", "audience", "subject", "email"})
    require(all(type(v) is str and 0 < len(v) <= 4096 for v in value["owner"].values()))
    require(all(type(value[k]) is int and 0 < value[k] < 2**63 for k in ("idle_seconds", "absolute_seconds")))
    return value


def binding_valid(binding):
    require(type(binding) is dict and set(binding) == BINDING_KEYS)
    require(binding["seed_ref"] == SEED_REF and binding["credential_id"] == "totp-next")
    for key in BINDING_KEYS - {"seed_ref", "credential_id"}:
        absolute(binding[key])
    roots = [binding[k] for k in ("auth_directory", "audit_directory", "release")]
    require(all(a != b and not a.startswith(b + "/") for i, a in enumerate(roots)
                for j, b in enumerate(roots) if i != j))
    require(all(not binding[k].startswith(root + "/") and binding[k] != root
                for k in ("config", "portable_factor") for root in roots))
    require(binding["config"] != binding["portable_factor"])


def release_manifest(tree, scanned):
    raw = tree.read("manifest.json")
    value = decode(raw)
    require(type(value) is dict and set(value) == {"build", "source_commit", "assembly", "files"})
    require(all(type(value[k]) is str and 0 < len(value[k]) <= 512 for k in ("build", "source_commit", "assembly")))
    require(type(value["files"]) is dict)
    files = value["files"]
    require(RELEASE_REQUIRED <= set(files) and len(files) < MAX_NODES)
    require(any(name.startswith("example-vault/") for name in files))
    require("manifest.json" not in files)
    require(set(scanned[0]) == set(files) | {"manifest.json"})
    require(set(scanned[1]) == parents(files))
    for name, expected in files.items():
        safe(name)
        require(type(expected) is dict and set(expected) == {"bytes", "sha256"})
        require(type(expected["bytes"]) is int and 0 <= expected["bytes"] <= MAX_FILE)
        require(type(expected["sha256"]) is str and re.fullmatch(r"[a-f0-9]{64}", expected["sha256"]))
        require(scanned[0][name]["size"] == expected["bytes"] and scanned[0][name]["sha256"] == expected["sha256"])
        if name.startswith("bin/"):
            require(scanned[0][name]["mode"] in (0o700, 0o755))
    return hashlib.sha256(raw).hexdigest()


def cipher(tree, name):
    raw = tree.read(name, MAX_CIPHER, private=True)
    require(64 <= len(raw) <= MAX_CIPHER and raw.startswith(AGE_HEADER))


def database(conn, version):
    deadline = time.monotonic() + DB_SECONDS
    conn.set_progress_handler(lambda: int(time.monotonic() >= deadline), 1000)
    require(conn.execute("PRAGMA user_version").fetchone() == (version,))
    require(conn.execute("PRAGMA quick_check(1)").fetchall() == [("ok",)])
    require(conn.execute("PRAGMA foreign_key_check").fetchone() is None)
    if version == 4:
        require(conn.execute("SELECT count(*) FROM auth_meta WHERE id=1 AND typeof(store_id)='text' AND length(store_id)=32 AND store_id NOT GLOB '*[^0-9a-f]*'").fetchone() == (1,))
        require(conn.execute("SELECT count(*) FROM totp_factor").fetchone() == (1,))
        require(conn.execute("SELECT count(*) FROM totp_factor WHERE id=1 AND seed_ref=? AND typeof(enrollment_json)='text' AND length(enrollment_json)>0 AND typeof(intent_receipt)='text' AND length(intent_receipt)>0 AND typeof(outcome_receipt)='text' AND length(outcome_receipt)>0 AND typeof(confirmed_at)='integer' AND confirmed_at>=0", (SEED_REF,)).fetchone() == (1,))
        for table in ("totp_pending", "recovery_pending"):
            require(conn.execute("SELECT count(*) FROM " + table).fetchone() == (0,))


def manifest_valid(value):
    require(type(value) is dict and set(value) == MANIFEST_KEYS)
    require(type(value["schema_version"]) is int and value["schema_version"] == 1 and value["state"] == "complete")
    require(type(value["snapshot_id"]) is str and re.fullmatch(r"[A-Za-z0-9-]{1,64}", value["snapshot_id"]))
    binding_valid(value["source_binding"])
    require(type(value["source_release_manifest_sha256"]) is str and re.fullmatch(r"[a-f0-9]{64}", value["source_release_manifest_sha256"]))
    files, dirs = value["files"], value["directories"]
    require(type(files) is dict and type(dirs) is dict and len(files) + len(dirs) <= MAX_NODES)
    require(BASE_FILES | {"release/manifest.json"} <= set(files))
    require(all(name in BASE_FILES or name.startswith("release/") for name in files))
    require(set(dirs) == parents(files))
    total = 0
    for name, item in files.items():
        safe(name)
        require(type(item) is dict and set(item) == {"sha256", "size", "mode"})
        require(type(item["size"]) is int and 0 <= item["size"] <= MAX_FILE)
        require(type(item["sha256"]) is str and re.fullmatch(r"[a-f0-9]{64}", item["sha256"]))
        modes = (0o700, 0o755) if name.startswith("release/bin/") else (0o600,)
        require(type(item["mode"]) is int and item["mode"] in modes)
        total += item["size"]
    require(total <= MAX_TOTAL)
    require(all(safe(name) and type(item) is dict and set(item) == {"mode"} and type(item["mode"]) is int and item["mode"] == 0o700 for name, item in dirs.items()))
