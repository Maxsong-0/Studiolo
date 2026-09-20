#!/usr/bin/env python3
"""仅在 T-087 私有恢复副本模拟三级吊销，验收 4401 接入方每次调用复核。
用法：python3 check-policy-revocation.py <stage-root>
直接插入隔离副本测试行并精确删除；不验证 L2 管理 API，不是生产吊销/恢复操作。
实例吊销不应影响新实例；类型与全局吊销应同时拒绝新会话。
"""
import argparse
import contextlib
from datetime import datetime, timezone
import http.client
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import sqlite3
import stat
import sys
import uuid

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve()
HELPER = HERE.with_name("check-agent-policy.py")
if not HELPER.is_file():
    HELPER = HERE.parents[2] / "scripts/check-agent-policy.py"
spec = importlib.util.spec_from_file_location("policy_probe", HELPER)
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)
require = probe.require
STAGE_PATTERN = re.compile(r"/home/maxsong/\.local/share/studiolo-m4-stage-t087-[A-Za-z0-9]+")
PRODUCTION_DB = Path("/home/maxsong/.local/share/studiolo-agent-policy/policy.sqlite3")
URL = "http://127.0.0.1:4401/mcp"
INSTANCE = re.compile(r"^类型 claude · 实例 ([0-7][0-9A-HJKMNP-TV-Z]{25}) · 注册 [^\n]+$", re.MULTILINE)
DENIALS = {"instance": "实例已被吊销", "type": "类型已被吊销", "global": "全局吊销生效中"}
COLUMNS = "seq, level, target, revoked_at, revoked_by, reason, released_at, released_by"


def private(path, directory=False):
    info = path.lstat()
    require((stat.S_ISDIR(info.st_mode) if directory else stat.S_ISREG(info.st_mode))
            and info.st_uid == os.getuid()
            and stat.S_IMODE(info.st_mode) == (0o700 if directory else 0o600)
            and (directory or info.st_nlink == 1), "阶段文件或目录必须为本人独占的私有实体")
    return info


def validate_stage(value):
    require(STAGE_PATTERN.fullmatch(value) is not None, "仅允许固定 T-087 阶段目录")
    root = Path(value)
    for ancestor in reversed((root, *root.parents)):
        require(stat.S_ISDIR(ancestor.lstat().st_mode), "阶段路径的祖先或自身不是实际目录")
    for directory in (root, root / "policy", root / "config"):
        private(directory, directory=True)
    db, token = root / "policy/policy.sqlite3", root / "config/claude.token"
    info = private(db)
    private(token)
    for suffix in ("-wal", "-shm", "-journal"):
        sidecar = Path(str(db) + suffix)
        if sidecar.exists() or sidecar.is_symlink():
            private(sidecar)
    production = PRODUCTION_DB.stat()
    require((info.st_dev, info.st_ino) != (production.st_dev, production.st_ino),
            "阶段数据库与生产数据库不能是同一文件")
    return db, token


def open_fixture(path):
    connection = sqlite3.connect(path.as_uri() + "?mode=rw", uri=True, timeout=5)
    try:
        require(connection.execute("PRAGMA application_id").fetchone() == (0x53504F4C,)
                and connection.execute("PRAGMA user_version").fetchone() == (1,), "阶段策略库格式不符")
        columns = [row[1] for row in connection.execute("PRAGMA table_info(policy_revocation)")]
        require(columns == COLUMNS.split(", "), "阶段吊销表字段不符")
        require(not connection.execute("SELECT 1 FROM sqlite_schema WHERE type='trigger'").fetchone(),
                "阶段库不得含额外触发器")
        require(connection.execute("PRAGMA quick_check(1)").fetchone() == ("ok",), "阶段库完整性检查失败")
        return connection
    except BaseException:
        connection.close()
        raise


@contextlib.contextmanager
def fixture_revocation(connection, level, target):
    """只删除自己插入、且八个字段仍全部匹配的记录，绝不清理已有吊销。"""
    stamp = datetime.now(timezone.utc).isoformat()
    fields = (level, target, stamp, "t087-isolated-fixture", "t087-" + uuid.uuid4().hex, None, None)
    row = None
    try:
        with connection:
            require(not connection.execute(
                "SELECT 1 FROM policy_revocation WHERE level=? AND target=? AND released_at IS NULL",
                (level, target)).fetchone(), "阶段目标已有吊销，禁止覆盖")
            inserted = connection.execute(
                "INSERT INTO policy_revocation (level,target,revoked_at,revoked_by,reason,released_at,released_by) "
                "VALUES (?,?,?,?,?,?,?)", fields)
            row = (inserted.lastrowid, *fields)
        yield
    finally:
        if row is not None:
            with connection:
                deleted = connection.execute(
                    "DELETE FROM policy_revocation WHERE " + " AND ".join(
                        name + " IS ?" for name in COLUMNS.split(", ")), row)
                require(deleted.rowcount == 1, "测试吊销行已变化或清理失败，阶段验收失败")


def denial_text(level):
    return "策略拒绝：" + DENIALS[level] + "；本次会话所有读取与写入全部拒绝"


def initialize(token, denied=None):
    client = probe.Client(URL, token)
    headers, response = client.rpc(1, "initialize", {
        "protocolVersion": probe.agent.PROTOCOL, "capabilities": {},
        "clientInfo": {"name": "studiolo-stage-revocation-check", "version": "1"}})
    sessions = headers.get_all("Mcp-Session-Id", [])
    require(len(sessions) == 1 and 0 < len(sessions[0]) <= 4096
            and all(33 <= ord(char) <= 126 for char in sessions[0]), "阶段会话头无效")
    client.session = sessions[0]
    result = response["result"]
    require(result.get("protocolVersion") == probe.agent.PROTOCOL
            and isinstance(result.get("serverInfo"), dict)
            and result["serverInfo"].get("name") == "studiolo-agent", "阶段服务身份或协议不符")
    instructions = result.get("instructions")
    instance, policy = None, None
    if denied:
        require(instructions == denial_text(denied), "新会话未精确拒绝或夹带正文")
    else:
        policy = probe.signpost(instructions)
        identities = INSTANCE.findall(instructions)
        require(len(identities) == 1, "路牌未给出唯一 claude 实例")
        instance = identities[0]
    client.post({"jsonrpc": "2.0", "method": "notifications/initialized"}, 202)
    return client, instance, policy


def read_note(client, denied=None):
    _, response = client.rpc(2 if denied is None else 3, "tools/call", {
        "name": "read_note", "arguments": {"path": probe.agent.INDEX_PATH}}, allow_error=True)
    if denied:
        error = response.get("error")
        require(set(response) == {"jsonrpc", "id", "error"} and isinstance(error, dict)
                and set(error) <= {"code", "message", "data"} and error.get("data") is None
                and error.get("code") == -32600 and error.get("message") == denial_text(denied),
                "既有会话未精确拒绝或夹带正文")
    else:
        result = response.get("result")
        require("error" not in response and isinstance(result, dict)
                and result.get("isError", False) is False, "阶段允许读取失败")
        content = result.get("content")
        require(isinstance(content, list) and len(content) == 1 and isinstance(content[0], dict)
                and content[0].get("type") == "text" and isinstance(content[0].get("text"), str),
                "阶段笔记读取格式无效")
        note = content[0]["text"]
        require(re.match(r"revision [0-9a-f]{64}\n", note) is not None
                and "Studiolo · 总览" in note, "阶段笔记正文或版本无效")


def check(stage_root):
    db, token_path = validate_stage(stage_root)
    with token_path.open("rb") as source:
        token_bytes = source.read(probe.MAX_TOKEN_BYTES + 1)
    require(len(token_bytes) <= probe.MAX_TOKEN_BYTES, "阶段凭据过长")
    token = token_bytes.decode("utf-8").strip()
    # 完整只读 present 门禁先于任何 fixture 写入；只保留数字/哈希摘要。
    summary = io.StringIO()
    with contextlib.redirect_stdout(summary):
        probe.check(URL, token)
    expected = json.loads(summary.getvalue())["effective_hash_prefix"]
    validate_stage(stage_root)
    with contextlib.closing(open_fixture(db)) as connection:
        for level in ("instance", "type", "global"):
            client, instance, policy = initialize(token)
            require(policy[2] == expected, "阶段策略版本发生变化")
            read_note(client)
            target = instance if level == "instance" else ("claude" if level == "type" else "*")
            with fixture_revocation(connection, level, target):
                read_note(client, denied=level)
                fresh, new_instance, new_policy = initialize(token, denied=None if level == "instance" else level)
                if level == "instance":
                    require(new_instance != instance and new_policy[2] == expected, "实例吊销越界或策略变化")
                    read_note(fresh)
                else:
                    read_note(fresh, denied=level)
            # 清理 fixture 后同时验证原会话可继续、全新会话可读，未声称生产实例可恢复。
            read_note(client)
            restored, _, policy = initialize(token)
            require(policy[2] == expected, "清理 fixture 后策略版本变化")
            read_note(restored)
    print(json.dumps({"fixture_levels_checked": 3, "existing_session_denials": 3,
                      "fresh_session_denials": 2, "unrelated_instances_allowed": 1,
                      "fixture_rows_cleaned": 3, "effective_hash_prefix": expected}, sort_keys=True))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage_root")
    args = parser.parse_args(argv)
    previous_umask = os.umask(0o077)
    try:
        check(args.stage_root)
    except probe.agent.CheckFailed as error:
        print(f"阶段吊销验收失败：{error}", file=sys.stderr)
        return 1
    except (OSError, ValueError, OverflowError, KeyError, sqlite3.Error,
            RecursionError, http.client.HTTPException):
        print("阶段吊销验收失败：阶段文件、数据库或 HTTP 响应不可用。", file=sys.stderr)
        return 1
    finally:
        os.umask(previous_umask)
    return 0


if __name__ == "__main__":
    sys.exit(main())
