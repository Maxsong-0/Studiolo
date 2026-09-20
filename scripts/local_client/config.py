"""Private, explicit local registrations; never accept executable paths over HTTP."""
import json
import os
from pathlib import Path
import re
import secrets
import stat
from urllib.parse import urlsplit


def private_read(path):
    path = Path(path)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_mode & 0o077:
            raise ValueError("配置和凭据文件必须为当前用户所有的 0600 普通文件")
        with os.fdopen(fd, "r") as stream:
            fd = -1
            value = stream.read(262145)
        if len(value) > 262144:
            raise ValueError("配置文件过大")
        return value
    finally:
        if fd >= 0:
            os.close(fd)


def private_create(path, content):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as stream:
        stream.write(content)


def identifier(value):
    if not isinstance(value, str) or not re.fullmatch(r"[a-zA-Z0-9_-]{1,100}", value):
        raise ValueError("标识格式无效")
    return value


def task_id(value):
    if not isinstance(value, str) or not re.fullmatch(r"[0-7][0-9A-HJKMNP-TV-Z]{25}", value):
        raise ValueError("工作项 ID 无效")
    return value


def load(path):
    cfg = json.loads(private_read(path))
    if cfg.get("version") != 1 or set(cfg) != {"version", "port", "origin", "state_directory", "pairing_token_file", "clients"}:
        raise ValueError("本机配置版本或字段无效")
    if type(cfg["port"]) is not int or not 1024 <= cfg["port"] <= 65535:
        raise ValueError("端口无效")
    origin = urlsplit(cfg["origin"])
    if (origin.scheme not in {"http", "https"} or not origin.hostname or origin.path
            or origin.query or origin.fragment or origin.username or origin.password
            or (origin.scheme == "http" and origin.hostname not in {"localhost", "127.0.0.1"})):
        raise ValueError("只接受明确的 Studiolo HTTPS 或本机开发 origin")
    state = Path(cfg["state_directory"])
    if not state.is_absolute() or state.is_symlink():
        raise ValueError("状态目录必须为绝对普通目录")
    state.mkdir(mode=0o700, parents=True, exist_ok=True)
    info = state.stat()
    if info.st_uid != os.getuid() or info.st_mode & 0o077:
        raise ValueError("状态目录必须为当前用户所有且权限 0700")
    cfg["state_directory"] = state
    secret = private_read(cfg["pairing_token_file"]).strip()
    if not re.fullmatch(r"[A-Za-z0-9_-]{32,128}", secret):
        raise ValueError("配对码格式无效")
    cfg["pairing_token"] = secret
    clients = cfg["clients"]
    if not isinstance(clients, list) or not 1 <= len(clients) <= 16:
        raise ValueError("须登记 1 到 16 个本机客户端")
    seen = set()
    for client in clients:
        allowed = {"id", "provider", "mode", "binary", "cwd", "project", "thread", "remote", "mcp_endpoint", "mcp_token_file", "bridge_token_file", "included_quota_only", "paid_usage_disabled"}
        if set(client) - allowed:
            raise ValueError("未知客户端字段")
        name = identifier(client["id"])
        if name in seen:
            raise ValueError("重复客户端标识")
        seen.add(name)
        if client["provider"] not in {"codex", "claude"} or client["mode"] != {"codex": "codex-queue", "claude": "claude-channel"}[client["provider"]]:
            raise ValueError("仅支持已登记的 Codex 会话与 Claude Channel")
        if client["project"] != "项目/Studiolo":
            raise ValueError("首批仅开放 Studiolo 项目写权限")
        for key in ("binary", "cwd"):
            p = Path(client[key])
            if not p.is_absolute() or not p.exists():
                raise ValueError("本机执行文件和目录必须已存在且为绝对路径")
        if not Path(client["cwd"]).is_dir() or not os.access(client["binary"], os.X_OK):
            raise ValueError("本机工作目录或执行文件无效")
        if client["provider"] == "codex" and not re.fullmatch(r"[0-9a-fA-F]{8}(?:-[0-9a-fA-F]{4}){3}-[0-9a-fA-F]{12}", client.get("thread", "")):
            raise ValueError("必须登记具体 Codex 会话 UUID")
        if client.get("remote") and (not client["remote"].startswith("unix:///") or "\x00" in client["remote"]):
            raise ValueError("只接受已登记的本机 Unix app-server")
        for key in ("mcp_token_file", "bridge_token_file"):
            if not Path(client[key]).is_absolute():
                raise ValueError("令牌路径必须为绝对路径")
            private_read(client[key])
        client["bridge_token"] = private_read(client["bridge_token_file"]).strip()
        if not re.fullmatch(r"[A-Za-z0-9_-]{32,128}", client["bridge_token"]):
            raise ValueError("本机客户端令牌无效")
    return cfg


def new_token(path):
    private_create(path, secrets.token_urlsafe(32) + "\n")
