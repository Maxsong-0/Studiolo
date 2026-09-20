#!/usr/bin/env python3
"""只读验收已经接入的 M4 策略平面；兼容模式、降级或策略拒绝均失败。
用法：python3 scripts/check-agent-policy.py <mcp_url> <token_file>
不调用提案、发布或吊销接口。读取仍会产生服务端正常实例登记与读取审计。
只输出计数及哈希；协议仅暴露 effective 哈希前 12 位，不能据此核对完整哈希。
"""
import argparse
import http.client
import importlib.util
import json
from pathlib import Path
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

# 明确加载相邻文件，复用协议、工具集合、拒绝标记、JSON/SSE 解析与封闭错误。
# 不执行旧 check()：它允许兼容模式，且 HTTP 读取未设长度上限。
sys.dont_write_bytecode = True
_spec = importlib.util.spec_from_file_location(
    "studiolo_mcp_check", Path(__file__).resolve().with_name("check-agent.py"))
agent = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(agent)
require = agent.require

TIMEOUT_SECONDS = 15
MAX_RESPONSE_BYTES = 4 * 1024 * 1024 + 2048
MAX_TOKEN_BYTES = 4096
# crates/supply/src/signpost/render.rs：第 3 项固定格式，不能仅凭 tools 清单判定 present。
CAPABILITY = re.compile(
    r"^## 3 能力\n读：([^\n]+)（能力集 v([1-9][0-9]{0,19}) ∩ 部署上限）；"
    r"写：黑板 (?:无（M5）|只读)、正文 (?:仅 patch（M5 起）|仅经预览的 patch|无)；"
    r"策略：(?:可提 desired（永不自动生效）|不可提 desired)；"
    r"策略版本 ([0-9a-f]{12})\n(?=## 4 待我做的事(?:\n|$))", re.MULTILINE)
SKILLS_HEADER = re.compile(
    r"# 策略版本 ([0-9a-f]{12})\n([0-9]{1,10}) 个 skill"
    r"（索引式候选；硬绑定见路牌第 5 项）\n")


def signpost(text):
    require(isinstance(text, str), "路牌缺失")
    require(not text.startswith(agent.POLICY_DENIED_PREFIX), "策略拒绝，不能验收为已启用")
    require("审计不可用" not in text, "路牌处于审计降级状态")
    require(agent.POLICY_ABSENT_MARK not in text, "策略平面未接入，不能验收兼容模式")
    require(text.startswith("# Studiolo 路牌（代次 "), "路牌标题无效")
    matches = list(CAPABILITY.finditer(text))
    require(len(matches) == 1 and text.count("\n## 3 能力\n") == 1,
            "路牌未明确声明策略能力及版本")
    match = matches[0]
    # 只在内存比较权限行与版本，不回显白名单、实例或路牌正文。
    return match[0], int(match[2]), match[3]


class Client:
    def __init__(self, base, token):
        parsed = urllib.parse.urlsplit(base)
        require(parsed.scheme in {"http", "https"} and parsed.hostname
                and parsed.username is None and parsed.password is None
                and not parsed.query and not parsed.fragment and parsed.port != 0
                and not any(ord(char) <= 32 or ord(char) == 127 for char in base),
                "地址必须是不含凭据、查询或片段的 HTTP(S) MCP 地址")
        require(0 < len(token) <= MAX_TOKEN_BYTES
                and all(33 <= ord(char) <= 126 for char in token),
                "凭据文件为空、过长或格式无效")
        self.base, self.session = base, None
        self.headers = {"Content-Type": "application/json",
                        "Accept": "application/json, text/event-stream",
                        "MCP-Protocol-Version": agent.PROTOCOL,
                        "User-Agent": "studiolo-policy-check/1", "Authorization": "Bearer " + token}
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), agent.NoRedirect())
        self.responses = 0

    def post(self, body, expected=200):
        headers = dict(self.headers)
        if self.session:
            headers["Mcp-Session-Id"] = self.session
        request = urllib.request.Request(self.base, data=json.dumps(body).encode(),
                                         headers=headers, method="POST")
        try:
            response = self.opener.open(request, timeout=TIMEOUT_SECONDS)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            self.responses += 1
            # 非成功正文完全不读；含 Location 的响应也不能导致第二次请求。
            require(response.code == expected and response.geturl() == self.base,
                    "HTTP 状态或响应地址不符合预期")
            if expected == 202:
                return response.headers, ""
            raw = response.read(MAX_RESPONSE_BYTES + 1)
            require(len(raw) <= MAX_RESPONSE_BYTES, "响应超过有界读取上限")
            return response.headers, raw.decode("utf-8")

    def rpc(self, request_id, method, params, *, allow_error=False):
        headers, raw = self.post({"jsonrpc": "2.0", "id": request_id,
                                  "method": method, "params": params})
        value = agent.payload(raw, headers.get("Content-Type", ""), request_id)
        require(isinstance(value, dict) and value.get("jsonrpc") == "2.0"
                and type(value.get("id")) is int and value["id"] == request_id,
                "JSON-RPC 响应格式或 id 不匹配")
        if not allow_error:
            require("error" not in value and isinstance(value.get("result"), dict),
                    "JSON-RPC 错误或缺少 result")
        return headers, value

    def call(self, request_id, name, arguments):
        # 固定本探针的可调用工具，防止后续误加写接口。
        require(name in {"handshake", "policy_read"}, "探针不允许调用此工具")
        _, value = self.rpc(request_id, "tools/call", {"name": name, "arguments": arguments})
        result = value["result"]
        require(result.get("isError", False) is False, "只读工具返回错误")
        content = result.get("content")
        require(isinstance(content, list) and bool(content)
                and all(isinstance(item, dict) and item.get("type") == "text"
                        and isinstance(item.get("text"), str) for item in content),
                "只读工具缺少文本结果")
        return "".join(item["text"] for item in content)


def check(base, token):
    client = Client(base, token)
    headers, value = client.rpc(1, "initialize", {
        "protocolVersion": agent.PROTOCOL, "capabilities": {},
        "clientInfo": {"name": "studiolo-policy-check", "version": "1"}})
    sessions = headers.get_all("Mcp-Session-Id", [])
    require(len(sessions) == 1 and 0 < len(sessions[0]) <= 4096
            and all(33 <= ord(char) <= 126 for char in sessions[0]), "初始化会话头无效")
    client.session = sessions[0]
    result = value["result"]
    require(result.get("protocolVersion") == agent.PROTOCOL, "协议版本不符合请求")
    info = result.get("serverInfo")
    require(isinstance(info, dict) and info.get("name") == "studiolo-agent", "服务身份无效")
    initial = signpost(result.get("instructions"))
    client.post({"jsonrpc": "2.0", "method": "notifications/initialized"}, 202)
    _, value = client.rpc(2, "tools/list", {})
    tools = value["result"].get("tools")
    require(isinstance(tools, list) and len(tools) == len(agent.EXPECTED_TOOLS)
            and all(isinstance(tool, dict) and isinstance(tool.get("name"), str) for tool in tools)
            and {tool["name"] for tool in tools} == agent.EXPECTED_TOOLS,
            "工具清单不符合当前 MCP 工具约定")
    current = signpost(client.call(3, "handshake", {}))
    require(initial == current, "两次握手的权限或可观测策略版本不一致")
    index = client.call(4, "policy_read", {"kind": "skills"})
    header = SKILLS_HEADER.match(index)
    require(header is not None, "skills 索引缺少有效策略版本或计数")
    require(header[1] == initial[2], "skills 与握手的可观测策略版本不一致")
    count = int(header[2])
    require(bool(index[header.end():]) == (count > 0), "skills 索引计数与空索引状态不符")
    # 沿用 check-agent.py 的固定禁止路径；必须是白名单拒绝，不能拿策略整体拒绝顶替。
    _, denied = client.rpc(5, "tools/call", {
        "name": "read_note", "arguments": {"path": "School/x.md"}}, allow_error=True)
    error = denied.get("error")
    require("result" not in denied and isinstance(error, dict) and error.get("code") == -32602
            and error.get("message") == "路径不在供给白名单内", "禁止路径未按白名单约定拒绝")
    print(json.dumps({"responses_checked": client.responses, "tools_checked": len(tools),
                      "forbidden_reads_rejected": 1, "capability_version": initial[1],
                      "skills_count": count, "effective_hash_prefix": initial[2]}, sort_keys=True))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mcp_url")
    parser.add_argument("token_file")
    args = parser.parse_args(argv)
    try:
        with open(args.token_file, "rb") as source:
            raw = source.read(MAX_TOKEN_BYTES + 1)
        require(len(raw) <= MAX_TOKEN_BYTES, "凭据文件超过读取上限")
        check(args.mcp_url, raw.decode("utf-8").strip())
    except agent.CheckFailed as error:
        print(f"策略验收失败：{error}", file=sys.stderr)
        return 1
    except (OSError, ValueError, OverflowError, RecursionError, http.client.HTTPException):
        # 底层错误可能反射 URL、头或正文，永不回显异常文本。
        print("策略验收失败：无法读取凭据或完成 HTTP/JSON 响应读取。", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
