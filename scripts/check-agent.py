#!/usr/bin/env python3
"""验收 studiolo-agent：初始化、七个 tools（四个只读＋两个策略平面＋黑板）与白名单拒绝，任一步不符即失败。
策略平面只做只读探测：按路牌区分「未接入必须拒绝」与「已接入必须读得到 skills 索引」；
永不调用 propose_desired——探针不得在被验收的服务里留下任何提案。
用法：python3 scripts/check-agent.py <mcp_url> <token 文件> [--no-token|--bad-token]
服务器上无仓库时：ssh 主机 python3 - <url> <token 文件> < scripts/check-agent.py。
只打印状态、长度与校验结论，不打印 token、会话 id、笔记正文或服务错误正文。
"""
import argparse
import http.client
import json
import sys
import urllib.error
import urllib.request

PROTOCOL = "2025-11-25"
INDEX_PATH = "项目/Studiolo/00-总览/INDEX.md"
READ_TOOLS = {"handshake", "list_entries", "read_note", "search"}
# M5 起 tools/list 恒为七个；未装配的策略或黑板调用失败关闭，清单必须恰好相等。
POLICY_TOOLS = {"policy_read", "propose_desired"}
BOARD_TOOLS = {"board"}
EXPECTED_TOOLS = READ_TOOLS | POLICY_TOOLS | BOARD_TOOLS
# 与 crates/supply/src/signpost/render.rs、crates/agent-server/src/server/policy.rs（「策略平面未接入」）与 server/session.rs（「策略拒绝：」）的固定文案逐字一致。
POLICY_ABSENT_MARK = "策略平面：未接入"
POLICY_ABSENT_ERROR = "策略平面未接入（M3 兼容模式）"
POLICY_DENIED_PREFIX = "策略拒绝："


class CheckFailed(Exception):
    """只携带本脚本生成的诊断，不包含服务响应或秘密。"""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        # urllib 默认会将 Authorization 带到另一主机；端点验收不跟随任何重定向。
        return None


def require(condition, message):
    if not condition:
        raise CheckFailed(message)


def payload(raw, content_type, request_id):
    try:
        if "text/event-stream" in content_type:
            for event in raw.replace("\r\n", "\n").split("\n\n"):
                data = "\n".join(line[5:].lstrip(" ") for line in event.splitlines()
                                 if line.startswith("data:"))
                if data:
                    value = json.loads(data)
                    if isinstance(value, dict) and value.get("id") == request_id:
                        return value
            raise CheckFailed("SSE 缺少对应请求的 JSON-RPC 响应")
        require("application/json" in content_type, "响应 Content-Type 不是 JSON 或 SSE")
        return json.loads(raw)
    except (ValueError, TypeError):
        raise CheckFailed("响应不是有效 JSON") from None


def check(base, token, mode):
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
               "MCP-Protocol-Version": PROTOCOL, "User-Agent": "studiolo-mcp-check/1"}
    if mode != "no-token":
        headers["Authorization"] = "Bearer " + token
    opener = urllib.request.build_opener(NoRedirect())

    def post(body, session=None):
        request_headers = dict(headers)
        if session:
            request_headers["Mcp-Session-Id"] = session
        request = urllib.request.Request(base, data=json.dumps(body).encode(),
                                         headers=request_headers, method="POST")
        try:
            with opener.open(request, timeout=30) as response:
                return response.status, response.headers, response.read().decode("utf-8")
        except urllib.error.HTTPError as error:
            # 不输出或解析非成功响应正文，它可能含反射的秘密或笔记。
            error.close()
            return error.code, error.headers, ""

    def rpc(request_id, method, params, session=None, *, allow_error=False):
        status, response_headers, raw = post({"jsonrpc": "2.0", "id": request_id,
                                             "method": method, "params": params}, session)
        print(f"{method}: HTTP {status}")
        require(status == 200, f"{method} 必须返回 HTTP 200")
        value = payload(raw, response_headers.get("Content-Type", ""), request_id)
        require(isinstance(value, dict) and value.get("jsonrpc") == "2.0"
                and value.get("id") == request_id, f"{method} 响应格式或 id 不匹配")
        if not allow_error:
            require("error" not in value and isinstance(value.get("result"), dict),
                    f"{method} 返回 JSON-RPC 错误或缺少 result")
        return response_headers, value

    initialize = {"protocolVersion": PROTOCOL, "capabilities": {},
                  "clientInfo": {"name": "studiolo-mcp-check", "version": "1"}}
    if mode:
        status, response_headers, _ = post({"jsonrpc": "2.0", "id": 1,
                                           "method": "initialize", "params": initialize})
        challenge = response_headers.get("WWW-Authenticate", "")
        print(f"initialize（阴性）: HTTP {status}")
        require(status == 401 and challenge.split(" ", 1)[0].lower() == "bearer",
                "无效凭据必须返回 HTTP 401 和 Bearer challenge")
        print("凭据拒绝验收通过。")
        return

    response_headers, value = rpc(1, "initialize", initialize)
    session = response_headers.get("Mcp-Session-Id")
    result = value["result"]
    require(bool(session), "initialize 缺少会话 id")
    require(result.get("protocolVersion") == PROTOCOL, "协议版本不符合请求")
    server_info = result.get("serverInfo")
    require(isinstance(server_info, dict) and server_info.get("name") == "studiolo-agent",
            "服务身份不符合 studiolo-agent")

    def signpost(text):
        # 策略拒绝是「应答但一项不给」：握手成功、路牌换成固定前缀的拒绝说明。单列出来，
        # 否则会被误报成审计降级；原因文本是服务端内容，不回显。
        require(not (isinstance(text, str) and text.startswith(POLICY_DENIED_PREFIX)),
                "握手被策略平面拒绝：探针所用类型已被吊销，或该类型尚未发布能力集")
        require(isinstance(text, str) and text.startswith("# Studiolo 路牌")
                and "审计不可用" not in text, "路牌缺失或处于审计降级状态")

    instructions = result.get("instructions")
    signpost(instructions)
    print(f"initialize: 会话已建立；路牌 {len(instructions)} 字符")
    status, _, _ = post({"jsonrpc": "2.0", "method": "notifications/initialized"}, session)
    print(f"initialized: HTTP {status}")
    require(status == 202, "initialized 必须返回 HTTP 202")
    _, value = rpc(2, "tools/list", {}, session)
    tools = value["result"].get("tools")
    require(isinstance(tools, list) and len(tools) == len(EXPECTED_TOOLS)
            and all(isinstance(tool, dict) and isinstance(tool.get("name"), str) for tool in tools)
            and {tool["name"] for tool in tools} == EXPECTED_TOOLS,
            "tools 清单不符合预期：应恰为四个只读工具加两个策略平面工具")
    print("tools/list: 四个只读工具、两个策略工具与黑板工具齐全")

    def call(request_id, name, arguments):
        _, value = rpc(request_id, "tools/call", {"name": name, "arguments": arguments}, session)
        result = value["result"]
        require(result.get("isError", False) is False, f"{name} 返回 tool 错误")
        content = result.get("content")
        require(isinstance(content, list) and bool(content)
                and all(isinstance(item, dict) and item.get("type") == "text"
                        and isinstance(item.get("text"), str) for item in content),
                f"{name} 缺少文本结果")
        text = "".join(item["text"] for item in content)
        print(f"{name}: 结果 {len(text)} 字符")
        return text

    signpost(call(3, "handshake", {}))
    note = call(4, "read_note", {"path": INDEX_PATH})
    require(note.startswith("revision ") and "Studiolo · 总览" in note,
            "INDEX.md 正文或 revision 不符合预期")
    entries = call(5, "list_entries", {"project": "项目/Studiolo"})
    require(f"note\t{INDEX_PATH}\t" in entries, "list_entries 未包含 INDEX.md")
    found = call(6, "search", {"project": "项目/Studiolo", "text": "00-总览 INDEX.md", "mode": "path"})
    require(f"{INDEX_PATH}:" in found, "search 未找到 INDEX.md")
    _, denied = rpc(7, "tools/call", {"name": "read_note", "arguments": {"path": "School/x.md"}},
                    session, allow_error=True)
    error = denied.get("error")
    require("result" not in denied and isinstance(error, dict) and error.get("code") == -32602
            and error.get("message") == "路径不在供给白名单内", "白名单外读取未按预期拒绝")
    if POLICY_ABSENT_MARK in instructions:
        _, refused = rpc(8, "tools/call", {"name": "policy_read", "arguments": {"kind": "skills"}},
                         session, allow_error=True)
        error = refused.get("error")
        require("result" not in refused and isinstance(error, dict) and error.get("code") == -32600
                and error.get("message") == POLICY_ABSENT_ERROR,
                "策略平面未接入时 policy_read 未按预期拒绝")
        print("policy_read: 策略平面未接入（M3 兼容模式），按预期拒绝")
    else:
        index = call(8, "policy_read", {"kind": "skills"})
        require(index.startswith("# 策略版本 "), "policy_read 未返回带策略版本的 skills 索引")
        print("policy_read: 策略平面已接入，skills 索引可读")
    print("完整供给链、白名单拒绝与策略平面探测验收通过。")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base_url")
    parser.add_argument("token_path")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--no-token", action="store_const", const="no-token", dest="mode")
    modes.add_argument("--bad-token", action="store_const", const="bad-token", dest="mode")
    args = parser.parse_args(argv)
    try:
        token = ""
        if args.mode == "bad-token":
            token = "sag1_" + "0" * 64
        elif args.mode != "no-token":
            with open(args.token_path, encoding="utf-8") as token_file:
                token = token_file.read().strip()
            require(bool(token), "token 文件为空")
        check(args.base_url, token, args.mode)
    except CheckFailed as error:
        print(f"验收失败：{error}", file=sys.stderr)
        return 1
    except (OSError, ValueError, http.client.HTTPException):
        # 底层异常可能回显 URL、header 或输入文件内容，不直接打印异常文本。
        print("验收失败：无法读取凭据或完成 HTTP/JSON 响应读取。", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
