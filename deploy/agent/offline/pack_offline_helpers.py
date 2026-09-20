"""离线包闭集清单与人侧引用校验；由同目录可信入口加载。"""
import datetime
import json
import os
import re
import stat
import unicodedata
FILE = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK

REPLICA_CHECK = '''
import os, re, sys
sys.path.insert(0, sys.argv[1])
from replica_common import open_root, open_dir, read_file, load_json, manifest, require
from replica_store import state_valid, verify_tree
root = open_root(sys.argv[2], private=True)
require(set(os.listdir(root)) == {"sync.lock", "control", "releases"})
read_file(root, "sync.lock", 4096)
control = open_dir(root, "control")
require(set(os.listdir(control)) == {"state.json"})
state = state_valid(load_json(read_file(control, "state.json", 8192)[0]))
require(state["status"] == "ready")
releases = open_dir(root, "releases")
names = os.listdir(releases)
require(0 < len(names) <= 64 and state["receipt"]["generation"] in names)
for name in names:
    require(re.fullmatch("[a-f0-9]{32}", name))
    generation = open_dir(releases, name)
    _, expected = manifest(read_file(generation, "manifest.json", 4 * 1024**2)[0])
    digest = verify_tree(generation, expected)
    if name == state["receipt"]["generation"]:
        require(digest == state["receipt"]["source_digest"])
    os.close(generation)
'''

def require(ok):
    if not ok:
        raise ValueError("validation_failed")

def safe(path):
    require(type(path) is str and 0 < len(path.encode("utf-8")) <= 8192)
    parts = path.split("/")
    gitkeep = parts[-1] == ".gitkeep" and path.startswith(("release/", "human/release/"))
    require(len(parts) <= 72 and all(p and not p.startswith(".") for p in (parts[:-1] if gitkeep else parts)))
    require("\\" not in path and unicodedata.normalize("NFC", path) == path)
    require(not any(unicodedata.category(c) == "Cc" for c in path))
    return path

def fp(st):
    return (st.st_dev, st.st_ino, st.st_mode, st.st_uid, st.st_gid,
            st.st_nlink, st.st_size, st.st_mtime_ns, st.st_ctime_ns)

def mode(path, actual=0o600):
    if path == "recovery/bin/studiolo-agent":
        return 0o700
    if path.startswith("human/release/bin/") and actual in (0o700, 0o755):
        return actual
    return 0o600

def load(tree):
    def pairs(items):
        value = {}
        for key, item in items:
            require(key not in value)
            value[key] = item
        return value
    fd = os.open("manifest.json", FILE, dir_fd=tree.fd)
    with os.fdopen(fd, "rb") as stream:
        st = os.fstat(fd)
        tree.node(st, False)
        require(stat.S_IMODE(st.st_mode) == 0o600 and st.st_size <= 32 * 1024**2)
        value = json.loads(stream.read(st.st_size + 1), object_pairs_hook=pairs, parse_constant=lambda _: require(False))
        require(fp(st) == fp(os.fstat(fd)) == fp(os.stat("manifest.json", dir_fd=tree.fd, follow_symlinks=False)))
    require(type(value) is dict and type(value.get("schema_version")) is int)
    version = value["schema_version"]
    require(version in (1, 2) and value["state"] == "complete")
    keys = {"schema_version", "state", "created_at", "directories", "files"}
    require(set(value) == keys | ({"snapshot_id"} if version == 2 else set()))
    if version == 2:
        require(type(value["snapshot_id"]) is str and re.fullmatch("[A-Za-z0-9-]{1,64}", value["snapshot_id"]))
    require(type(value["created_at"]) is str and re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", value["created_at"]))
    datetime.datetime.strptime(value["created_at"], "%Y-%m-%dT%H:%M:%SZ")
    require(type(value["directories"]) is dict and type(value["files"]) is dict)
    require(all(safe(p) and type(v) is dict and set(v) == {"mode"} and type(v["mode"]) is int and v["mode"] == 0o700
                for p, v in value["directories"].items()))
    for name, item in value["files"].items():
        safe(name)
        require(type(item) is dict and set(item) == {"sha256", "size", "mode"})
        require(type(item["size"]) is int and item["size"] >= 0 and type(item["mode"]) is int and item["mode"] == mode(name, item["mode"]))
        require(type(item["sha256"]) is str and re.fullmatch("[a-f0-9]{64}", item["sha256"]))
    return value

def human_id(tree):
    fd = os.open("manifest.json", FILE, dir_fd=tree.fd)
    with os.fdopen(fd, "rb") as stream:
        before = os.fstat(fd)
        tree.node(before, False)
        require(stat.S_IMODE(before.st_mode) == 0o600 and before.st_size <= 4 * 1024**2)
        value = json.loads(stream.read(before.st_size + 1))
        require(fp(before) == fp(os.fstat(fd)) == fp(os.stat("manifest.json", dir_fd=tree.fd, follow_symlinks=False)))
    # 完整闭集/重复键/内容验证先由独立可信 human_snapshot 验证器执行。
    result = value["snapshot_id"]
    require(type(result) is str and re.fullmatch("[A-Za-z0-9-]{1,64}", result))
    return result
