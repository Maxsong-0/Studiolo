#!/usr/bin/env python3
"""核对 studiolo-agent 的连接器入口 /connect/v1/*（ADR-046）：只看状态码、头和条数，任一步不符即失败。
用法：python3 scripts/check-connector.py <服务地址> <token 文件> board|status [--project 项目/…]… [--no-token|--bad-token]
例：  python3 scripts/check-connector.py http://127.0.0.1:4400 ~/homelab-board.token board --project 项目/Studiolo-Docs
服务器上无仓库时：ssh 主机 python3 - <地址> <token 文件> board < scripts/check-connector.py。

- board：看黑板那张令牌。列表 200（给了 --project 就核名单恰好相等）、带 ETag 再问 304 或内容真变了、推状态 403。
- status：推状态那张令牌。只推一份故意不合的快照，预期 422：令牌、权限、到人侧的路都核到了，现有状态一条不动；看黑板 403。
- 服务地址是本机回环时，另核这张令牌开 MCP 会话、GET /read/handshake 一项内容都拿不到；外面只放行 /connect/，不核这两样。
- --no-token／--bad-token：只核 401，不读 token 文件。

应答没有 Studiolo-Connect 头、或带 cf-mitigated 头，就不是 Studiolo 回的：报「被边缘拦了」、退出码 3，不说成令牌错或黑板空。
不解质询、不重试、不跟重定向。不打印 token、应答正文和工作项内容。
"""
import argparse
import http.client
import ipaddress
import json
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

# 自己的标识：默认的 Python-urllib 在边缘另有被拦的先例（1010）。
USER_AGENT = "studiolo-connector-check/1"
MAX_BODY = 2 * 1024 * 1024
# 与 crates/agent-server/src/server/session.rs 的固定前缀逐字一致。
POLICY_DENIED_PREFIX = "策略拒绝："
ETAG = re.compile(r'(W/)?"[0-9a-f]{32}"\Z')
PROJECT_KEYS = {"project", "items", "finished", "truncated"}


class CheckFailed(Exception):
    """只携带本脚本生成的诊断，不包含服务应答或秘密。"""


class EdgeBlocked(Exception):
    """应答不是 Studiolo 回的。"""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # urllib 默认会把 Authorization 带到另一主机；这里一律不跟。
        return None


def require(condition, message):
    if not condition:
        raise CheckFailed(message)


def loopback(host):
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def origin(base):
    """只收 https，或本机回环上的 http：令牌不明文过网。返回（地址，是不是回环）。"""
    parts = urllib.parse.urlsplit(base)
    host = parts.hostname or ""
    local = loopback(host)
    require(parts.scheme == "https" or (parts.scheme == "http" and local),
            "服务地址只收 https，或本机回环上的 http")
    require(bool(host) and parts.path in ("", "/") and not parts.query and not parts.fragment
            and parts.username is None and parts.password is None,
            "服务地址只写到主机和端口，不带路径、参数和口令")
    return f"{parts.scheme}://{parts.netloc}", local


class Client:
    def __init__(self, base, token):
        self.base, self.token = base, token
        self.opener = urllib.request.build_opener(NoRedirect())

    def send(self, method, path, body=None, headers=None):
        """一次请求；只有 2xx 才读正文，且有上限。"""
        sent = {"User-Agent": USER_AGENT, "Accept": "application/json", **(headers or {})}
        if self.token:
            sent["Authorization"] = "Bearer " + self.token
        data = None
        if body is not None:
            data = json.dumps(body).encode()
            sent["Content-Type"] = "application/json"
        request = urllib.request.Request(self.base + path, data=data, headers=sent, method=method)
        try:
            with self.opener.open(request, timeout=30) as response:
                status, received, raw = response.status, response.headers, response.read(MAX_BODY + 1)
        except urllib.error.HTTPError as error:
            error.close()
            status, received, raw = error.code, error.headers, b""
        # 放在判断状态码之前：质询页也是 403，不能当成「没有权限」。
        if received.get("cf-mitigated") is not None:
            raise EdgeBlocked(f"{method} {path}: HTTP {status}，应答带 cf-mitigated 头")
        require(len(raw) <= MAX_BODY, f"{path} 的应答超过上限")
        print(f"{method} {path}: HTTP {status}")
        return status, received, raw

    def connect(self, method, path, body=None, headers=None):
        status, received, raw = self.send(method, path, body, headers)
        if received.get("Studiolo-Connect") != "1":
            raise EdgeBlocked(f"{method} {path}: HTTP {status}，应答没有 Studiolo-Connect 头")
        require("no-store" in (received.get("Cache-Control") or ""), f"{path} 缺少 no-store")
        return status, received, raw


def parse_list(raw):
    try:
        value = json.loads(raw)
    except ValueError:
        raise CheckFailed("列表不是有效 JSON") from None
    projects = value.get("projects") if isinstance(value, dict) else None
    require(isinstance(projects, list) and set(value) == {"generated_at", "projects"}, "列表字段不符合契约")
    for project in projects:
        require(isinstance(project, dict) and set(project) == PROJECT_KEYS
                and isinstance(project["project"], str) and isinstance(project["items"], list)
                and type(project["finished"]) is int and type(project["truncated"]) is bool,
                "列表里的项目字段不符合契约")
    return projects


def check_board(client, expected):
    status, received, raw = client.connect("GET", "/connect/v1/board")
    require(status == 200, "看黑板必须返回 HTTP 200")
    projects = parse_list(raw)
    names = [project["project"] for project in projects]
    require(len(names) == len(set(names)), "同一个项目出现两次")
    if expected:
        missing, extra = set(expected) - set(names), set(names) - set(expected)
        require(not missing and not extra, f"名单不符：少 {len(missing)} 个、多 {len(extra)} 个项目")
    etag = received.get("ETag") or ""
    require(bool(ETAG.match(etag)), "列表缺少 ETag 或形状不对")
    print(f"board: {len(projects)} 个项目，{sum(len(project['items']) for project in projects)} 条没结束的")
    status, received, _ = client.connect("GET", "/connect/v1/board", headers={"If-None-Match": etag})
    changed = status == 200 and received.get("ETag") not in (None, etag)
    require(status == 304 or changed, "带上次的 ETag 再问，必须是 304（或内容真的变了）")
    status, _, _ = client.connect("PUT", "/connect/v1/sessions", body={"seq": 0, "sessions": []})
    require(status == 403, "看黑板的令牌推状态必须返回 HTTP 403")


def check_status(client):
    # 多一个字段整份不收：不会盖掉、也不会清空现有状态。
    body = {"seq": 0, "sessions": [], "probe": True}
    status, _, _ = client.connect("PUT", "/connect/v1/sessions", body=body)
    require(status == 422, "故意不合的快照必须返回 HTTP 422（403＝没这个权限，503＝到不了人侧）")
    status, _, _ = client.connect("GET", "/connect/v1/board")
    require(status == 403, "推状态的令牌看黑板必须返回 HTTP 403")


def check_nothing_else(client):
    """这张令牌在 MCP 和 /read 上一项内容都拿不到。"""
    status, _, _ = client.send("GET", "/read/handshake")
    require(status == 403, "连接器令牌读 /read/handshake 必须返回 HTTP 403")
    body = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-11-25", "capabilities": {},
                       "clientInfo": {"name": "studiolo-connector-check", "version": "1"}}}
    status, _, raw = client.send("POST", "/mcp", body=body,
                                 headers={"Accept": "application/json, text/event-stream"})
    require(status == 200, "连接器令牌开 MCP 会话应握手成功但按策略拒绝")
    require(instructions(raw).startswith(POLICY_DENIED_PREFIX), "连接器令牌的 MCP 握手没有按策略拒绝")
    print("mcp: 握手按策略拒绝，不给路牌")


def instructions(raw):
    """握手说明；单条 JSON 或 SSE 都收，找不到就是空串。"""
    text = raw.decode("utf-8", "replace")
    chunks = [text] + [line[5:] for line in text.replace("\r\n", "\n").split("\n") if line.startswith("data:")]
    for chunk in chunks:
        try:
            found = json.loads(chunk)["result"]["instructions"]
        except (ValueError, KeyError, TypeError):
            continue
        if isinstance(found, str):
            return found
    return ""


def check_denied(client):
    for method, path in (("GET", "/connect/v1/board"), ("PUT", "/connect/v1/sessions")):
        status, received, _ = client.connect(method, path, body={} if method == "PUT" else None)
        challenge = received.get("WWW-Authenticate") or ""
        require(status == 401 and challenge.split(" ", 1)[0].lower() == "bearer",
                "无效凭据必须返回 HTTP 401 和 Bearer challenge")


def read_token(path):
    with open(path, encoding="utf-8") as token_file:
        token = token_file.read().strip()
    require(bool(token) and token.isascii() and token.isprintable() and " " not in token, "token 文件为空或格式不对")
    return token


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("base_url")
    parser.add_argument("token_path")
    parser.add_argument("entry", choices=("board", "status"))
    parser.add_argument("--project", action="append", default=[], help="board 应恰好列出的项目，可重复")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--no-token", action="store_const", const="no-token", dest="mode")
    modes.add_argument("--bad-token", action="store_const", const="bad-token", dest="mode")
    args = parser.parse_args(argv)
    try:
        base, local = origin(args.base_url)
        if args.mode:
            check_denied(Client(base, "sag1_" + "0" * 64 if args.mode == "bad-token" else ""))
            print("凭据拒绝核对通过。")
            return 0
        client = Client(base, read_token(args.token_path))
        if args.entry == "board":
            check_board(client, args.project)
        else:
            check_status(client)
        if local:
            check_nothing_else(client)
        else:
            print("不是本机回环：没核 MCP 和 /read（外面只放行 /connect/）。")
    except EdgeBlocked as blocked:
        print(f"被边缘拦了：{blocked}。这不是 Studiolo 的答复，和令牌、黑板内容无关。", file=sys.stderr)
        return 3
    except CheckFailed as error:
        print(f"核对失败：{error}", file=sys.stderr)
        return 1
    except (OSError, ValueError, http.client.HTTPException):
        # 底层异常可能回显 URL、头或文件内容，不打印异常文本。
        print("核对失败：读不到凭据，或连不上、读不完应答。", file=sys.stderr)
        return 1
    print("连接器核对通过。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
