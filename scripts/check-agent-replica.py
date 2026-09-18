#!/usr/bin/env python3
"""验收线上副本：initialize 与四个只读 MCP 工具必须报告同一来源回执和预期新鲜度。
tools/list 须恰为 check-agent.py 的 EXPECTED_TOOLS（M4 起六个）；两个策略平面工具不带副本回执，不在此比对。
用法：python3 scripts/check-agent-replica.py <mcp_url> <token 文件> [--expected ready|stale]
只打印已验证的新鲜度字段，不打印凭据、会话 id、笔记正文或服务错误正文。
同目录需要 check-agent.py；本脚本不会同步、修改副本或重启服务。
"""
import argparse
from datetime import datetime, timezone
import http.client
import importlib.util
import json
from pathlib import Path
import re
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("agent_check", Path(__file__).with_name("check-agent.py"))
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)
require = CHECK.require
MAX_RESPONSE_BYTES = 16 * 1024 * 1024
FRESHNESS = re.compile(
    r"副本状态 (?P<status>ready|stale)；stale=(?P<stale>true|false)；"
    r"generation=(?P<generation>[0-9a-f]{32})；source_digest=(?P<source_digest>[0-9a-f]{64})；"
    r"source_observed_at_ms=(?P<source_observed_at_ms>[0-9]{1,16})；"
    r"received_at_ms=(?P<received_at_ms>[0-9]{1,16})；age_secs=(?P<age_secs>[0-9]{1,16})；"
    r"max_age_secs=(?P<max_age_secs>[0-9]{1,5})"
)


class ReceiptChanged(CHECK.CheckFailed):
    """已有效的来源回执在连续读取中切换；只允许对这一类错误有限重试。"""


def freshness(text, name, expected):
    """只认协议规定位置的元数据，正文中仿造的新鲜度不能充当回执。"""
    require(isinstance(text, str), f"{name} 缺少文本")
    lines = text.splitlines()
    line_index = 1 if name in {"initialize", "handshake", "read_note"} else 0
    if name in {"initialize", "handshake"}:
        require(text.startswith("# Studiolo 路牌"), f"{name} 路牌缺失或不可用")
    elif name == "read_note":
        require(bool(lines) and re.fullmatch(r"revision [0-9a-f]+", lines[0]),
                "read_note 缺少 revision")
    match = FRESHNESS.fullmatch(lines[line_index]) if len(lines) > line_index else None
    require(match is not None, f"{name} 缺少有效副本新鲜度或报告同步失败")
    value = match.groupdict()
    for field in ("source_observed_at_ms", "received_at_ms", "age_secs", "max_age_secs"):
        value[field] = int(value[field])
    require(value["status"] == expected and value["stale"] == str(expected == "stale").lower(),
            f"{name} 新鲜度不符合预期状态")
    require(0 < value["source_observed_at_ms"] <= value["received_at_ms"] <= int(time.time() * 1000) + 30_000,
            f"{name} 来源或接收时间不合法")
    require(60 <= value["max_age_secs"] <= 86_400, f"{name} 新鲜度阈值不合法")
    # 服务以毫秒判断 stale，而 age_secs 向下取整；阈值秒本身可能出现两种状态。
    age_ok = (value["age_secs"] <= value["max_age_secs"] if expected == "ready"
              else value["age_secs"] >= value["max_age_secs"])
    require(age_ok, f"{name} 副本年龄与状态矛盾")
    return value


class Client:
    def __init__(self, url, token):
        parsed = urllib.parse.urlsplit(url)
        require(parsed.scheme in {"http", "https"} and parsed.hostname and not parsed.username
                and not parsed.password and not parsed.fragment and not parsed.query,
                "MCP URL 必须为不含凭据、查询或片段的 HTTP(S) 地址")
        self.url, self.session, self.request_id = url, None, 0
        self.headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
                        "MCP-Protocol-Version": CHECK.PROTOCOL, "Authorization": "Bearer " + token,
                        "User-Agent": "studiolo-replica-check/1"}
        self.opener = urllib.request.build_opener(CHECK.NoRedirect())

    def rpc(self, method, params, notification=False):
        self.request_id += 1
        body = {"jsonrpc": "2.0", "method": method, "params": params}
        if not notification:
            body["id"] = self.request_id
        headers = dict(self.headers)
        if self.session:
            headers["Mcp-Session-Id"] = self.session
        request = urllib.request.Request(self.url, data=json.dumps(body).encode(), headers=headers, method="POST")
        try:
            with self.opener.open(request, timeout=30) as response:
                require(response.status == (202 if notification else 200), f"{method} HTTP 状态不符合预期")
                if notification:
                    return None
                raw = response.read(MAX_RESPONSE_BYTES + 1)
                require(len(raw) <= MAX_RESPONSE_BYTES, f"{method} 响应超过验收读取上限")
                value = CHECK.payload(raw.decode("utf-8"), response.headers.get("Content-Type", ""), self.request_id)
                if method == "initialize":
                    self.session = response.headers.get("Mcp-Session-Id")
        except urllib.error.HTTPError as error:
            status = error.code
            error.close()
            raise CHECK.CheckFailed(f"{method} HTTP {status}，未读取错误正文") from None
        require(isinstance(value, dict) and value.get("jsonrpc") == "2.0"
                and value.get("id") == self.request_id and "error" not in value
                and isinstance(value.get("result"), dict), f"{method} JSON-RPC 响应无效或失败")
        return value["result"]

    def call(self, name, arguments):
        result = self.rpc("tools/call", {"name": name, "arguments": arguments})
        require(result.get("isError", False) is False, f"{name} 返回 tool 错误")
        content = result.get("content")
        require(isinstance(content, list) and bool(content)
                and all(isinstance(item, dict) and item.get("type") == "text"
                        and isinstance(item.get("text"), str) for item in content), f"{name} 缺少文本结果")
        return "".join(item["text"] for item in content)


def check(url, token, expected):
    client = Client(url, token)
    initial = client.rpc("initialize", {"protocolVersion": CHECK.PROTOCOL, "capabilities": {},
                                       "clientInfo": {"name": "studiolo-replica-check", "version": "1"}})
    require(bool(client.session) and initial.get("protocolVersion") == CHECK.PROTOCOL
            and isinstance(initial.get("serverInfo"), dict)
            and initial["serverInfo"].get("name") == "studiolo-agent", "initialize 会话、协议或服务身份无效")
    reference = freshness(initial.get("instructions"), "initialize", expected)
    client.rpc("notifications/initialized", {}, notification=True)
    tools = client.rpc("tools/list", {}).get("tools")
    require(isinstance(tools, list) and len(tools) == len(CHECK.EXPECTED_TOOLS)
            and all(isinstance(tool, dict) and isinstance(tool.get("name"), str) for tool in tools)
            and {tool["name"] for tool in tools} == CHECK.EXPECTED_TOOLS, "tools 清单不符合预期（应恰为四个只读加两个策略平面）")
    ages = [reference["age_secs"]]
    for name, arguments in (
        ("handshake", {}), ("read_note", {"path": CHECK.INDEX_PATH}),
        ("list_entries", {"project": "项目/Studiolo"}),
        ("search", {"project": "项目/Studiolo", "text": "00-总览 INDEX.md", "mode": "path"}),
    ):
        text = client.call(name, arguments)
        value = freshness(text, name, expected)
        if not all(value[key] == reference[key] for key in reference if key != "age_secs"):
            raise ReceiptChanged("验收期间来源换批，请重新验收")
        ages.append(value["age_secs"])
        if name == "read_note":
            require("Studiolo · 总览" in text, "read_note 未返回预期 INDEX")
        elif name == "list_entries":
            require(f"note\t{CHECK.INDEX_PATH}\t" in text, "list_entries 缺少 INDEX")
        elif name == "search":
            require(f"{CHECK.INDEX_PATH}:" in text, "search 缺少 INDEX")
    summary = {key: value for key, value in reference.items() if key != "age_secs"}
    summary.update({"check": "passed", "responses_checked": 5, "age_secs_min": min(ages), "age_secs_max": max(ages),
                    "source_observed_at_utc": datetime.fromtimestamp(
                        reference["source_observed_at_ms"] / 1000, timezone.utc).isoformat()})
    print(json.dumps(summary, ensure_ascii=False, sort_keys=True))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("base_url")
    parser.add_argument("token_path")
    parser.add_argument("--expected", choices=("ready", "stale"), default="ready")
    args = parser.parse_args(argv)
    try:
        with open(args.token_path, encoding="utf-8") as token_file:
            token = token_file.read(4097).strip()
        require(bool(token) and len(token) <= 4096 and not any(char.isspace() for char in token),
                "token 文件为空、过长或格式无效")
        check(args.base_url, token, args.expected)
    except ReceiptChanged:
        print("副本验收中来源换批，请重试。", file=sys.stderr)
        return 2
    except CHECK.CheckFailed as error:
        print(f"副本验收失败：{error}", file=sys.stderr)
        return 1
    except (OSError, ValueError, OverflowError, http.client.HTTPException):
        print("副本验收失败：无法读取凭据或完成 HTTP/JSON 响应读取。", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
