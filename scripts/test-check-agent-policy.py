#!/usr/bin/env python3
"""策略上线探针的隔离 loopback HTTP 契约；只使用临时凭据和假服务，不接真实服务。"""
import contextlib
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import threading
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location(
    "check_agent_policy", Path(__file__).resolve().with_name("check-agent-policy.py"))
probe = importlib.util.module_from_spec(spec)
spec.loader.exec_module(probe)

TOKEN = "fixture-token-never-print"
SESSION = "fixture-session-never-print"
PRIVATE = "fixture-private-body-never-print"
HASH = "0123456789ab"


def signpost(hash_prefix=HASH, version=1):
    return ("# Studiolo 路牌（代次 1，2026-09-20）\n## 1 我是谁\n" + SESSION
            + "\n## 2 项目状态\n" + PRIVATE + "\n## 3 能力\n"
            + f"读：项目/Studiolo（能力集 v{version} ∩ 部署上限）；写：黑板 只读、"
            + f"正文 仅经预览的 patch；策略：可提 desired（永不自动生效）；策略版本 {hash_prefix}\n"
            + "## 4 待我做的事\n无\n## 5 硬约束\n无\n## 6 继续拉的入口\npolicy_read\n")


def result(request_id, value):
    return {"jsonrpc": "2.0", "id": request_id, "result": value}


def tool(request_id, text):
    return result(request_id, {"content": [{"type": "text", "text": text}]})


def reply(value=None, *, status=200, headers=None, raw=None):
    return {"status": status, "headers": {"Content-Type": "application/json", **(headers or {})},
            "body": raw if raw is not None else json.dumps(value, ensure_ascii=False).encode()}


def happy_replies():
    return [
        reply(result(1, {"protocolVersion": probe.agent.PROTOCOL,
                         "serverInfo": {"name": "studiolo-agent"}, "instructions": signpost()}),
              headers={"Mcp-Session-Id": SESSION}),
        reply(status=202, raw=b""),
        reply(result(2, {"tools": [{"name": name} for name in sorted(probe.agent.EXPECTED_TOOLS)]})),
        reply(tool(3, signpost())),
        reply(tool(4, f"# 策略版本 {HASH}\n1 个 skill（索引式候选；硬绑定见路牌第 5 项）\n"
                      + f"fixture-skill\tv1\t{PRIVATE}\tfixture-trigger")),
        reply({"jsonrpc": "2.0", "id": 5,
               "error": {"code": -32602, "message": "路径不在供给白名单内"}}),
    ]


def change_result(replies, index, change):
    value = json.loads(replies[index]["body"])
    change(value["result"])
    replies[index]["body"] = json.dumps(value, ensure_ascii=False).encode()


class Server(ThreadingHTTPServer):
    daemon_threads = True

    def handle_error(self, request, client_address):
        # 过长响应测试会由客户端提前关闭连接；不将请求资料写进测试日志。
        pass


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        raw = self.rfile.read(int(self.headers["Content-Length"]))
        self.server.requests.append((self.path, dict(self.headers), json.loads(raw)))
        index = len(self.server.requests) - 1
        response = self.server.replies[index] if index < len(self.server.replies) else reply(status=500)
        self.send_response(response["status"])
        for key, value in response["headers"].items():
            self.send_header(key, value)
        self.send_header("Content-Length", str(len(response["body"])))
        self.end_headers()
        try:
            self.wfile.write(response["body"])
        except (BrokenPipeError, ConnectionResetError):
            pass

    def do_GET(self):
        # 能抓到 urllib 跟随 301/302/303 后发出的 GET。
        self.server.requests.append((self.path, dict(self.headers), {"method": "GET"}))
        self.send_response(500)
        self.end_headers()


class PolicyProbeTests(unittest.TestCase):
    def run_probe(self, replies, token=TOKEN):
        with tempfile.TemporaryDirectory(prefix="studiolo-policy-probe-") as directory:
            token_file = Path(directory) / "token"
            token_file.write_text(token, encoding="utf-8")
            token_file.chmod(0o600)
            with Server(("127.0.0.1", 0), Handler) as server:
                server.replies, server.requests = copy.deepcopy(replies), []
                port = server.server_address[1]
                for response in server.replies:
                    if "Location" in response["headers"]:
                        response["headers"]["Location"] = f"http://127.0.0.1:{port}/redirected"
                worker = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01})
                worker.start()
                output = io.StringIO()
                try:
                    with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
                        code = probe.main([f"http://127.0.0.1:{port}/mcp", str(token_file)])
                finally:
                    server.shutdown()
                    worker.join(timeout=2)
                self.assertFalse(worker.is_alive())
                text = output.getvalue()
                for private in (TOKEN, SESSION, PRIVATE):
                    self.assertNotIn(private, text)
                for _, _, request in server.requests:
                    if request.get("method") == "tools/call":
                        self.assertIn(request["params"]["name"], {"handshake", "policy_read", "read_note"})
                if code:
                    self.assertEqual(code, 1)
                    self.assertTrue(text.startswith("策略验收失败："))
                    self.assertEqual(len(text.splitlines()), 1)
                return code, server.requests, text

    def test_present_json_and_sse_pass_without_content_or_writes(self):
        for sse in (False, True):
            with self.subTest(sse=sse):
                replies = happy_replies()
                if sse:
                    for response in replies:
                        if response["status"] == 200:
                            response["headers"]["Content-Type"] = "text/event-stream"
                            response["body"] = b'data: {"method":"notifications/ping"}\n\n' + \
                                b"data: " + response["body"] + b"\n\n"
                code, requests, output = self.run_probe(replies)
                self.assertEqual(code, 0)
                summary = json.loads(output)
                self.assertEqual(summary, {"responses_checked": 6, "tools_checked": 7,
                                          "forbidden_reads_rejected": 1, "capability_version": 1,
                                          "skills_count": 1, "effective_hash_prefix": HASH})
                self.assertEqual(len(requests), 6)
                self.assertEqual([request[2]["method"] for request in requests],
                                 ["initialize", "notifications/initialized", "tools/list",
                                  "tools/call", "tools/call", "tools/call"])
                for index, (path, headers, request) in enumerate(requests):
                    self.assertEqual(path, "/mcp")
                    self.assertEqual(headers["Authorization"], "Bearer " + TOKEN)
                    self.assertEqual(headers.get("Mcp-Session-Id"), SESSION if index else None)
                self.assertEqual(requests[-1][2]["params"],
                                 {"name": "read_note", "arguments": {"path": "School/x.md"}})

    def test_read_only_probe_rejects_unexpected_board_write_grant(self):
        replies = happy_replies()
        change_result(replies, 0, lambda value: value.update(
            instructions=signpost().replace("黑板 只读", "黑板 可写")))
        self.assertEqual(self.run_probe(replies)[0], 1)

    def test_empty_skills_index_is_valid(self):
        replies = happy_replies()
        replies[4] = reply(tool(4, f"# 策略版本 {HASH}\n0 个 skill（索引式候选；硬绑定见路牌第 5 项）\n"))
        self.assertEqual(self.run_probe(replies)[0], 0)

    def test_absent_denied_and_degraded_signposts_fail(self):
        for bad in ("# Studiolo 路牌\n策略平面：未接入", "策略拒绝：" + PRIVATE,
                    "审计不可用：" + PRIVATE, signpost().replace("## 3 能力", "## 3 未知")):
            for index in (0, 3):
                with self.subTest(index=index, bad=bad[:12]):
                    replies = happy_replies()
                    if index == 0:
                        change_result(replies, index, lambda value: value.update(instructions=bad))
                    else:
                        replies[index] = reply(tool(3, bad))
                    self.assertEqual(self.run_probe(replies)[0], 1)

    def test_http_auth_failure_and_redirect_never_pass_or_follow(self):
        for status in (401, 403, 301, 302, 303, 307, 308, 500):
            with self.subTest(status=status):
                replies = [reply(status=status, headers={"Location": "/redirected"},
                                 raw=(TOKEN + SESSION + PRIVATE).encode())]
                code, requests, _ = self.run_probe(replies)
                self.assertEqual(code, 1)
                self.assertEqual(len(requests), 1)

    def test_oversized_malformed_and_rpc_errors_do_not_reflect_body(self):
        for raw in ((TOKEN + SESSION + PRIVATE).encode(), b"[", b"\xff",
                    (TOKEN + PRIVATE).encode() + b"x" * probe.MAX_RESPONSE_BYTES):
            with self.subTest(length=len(raw)):
                replies = happy_replies()
                replies[4] = reply(raw=raw)
                self.assertEqual(self.run_probe(replies)[0], 1)
        for index in (0, 2, 3, 4, 5):
            with self.subTest(index=index):
                replies = happy_replies()
                request_id = json.loads(replies[index]["body"])["id"]
                replies[index] = reply({"jsonrpc": "2.0", "id": request_id,
                                        "error": {"code": -32600, "message": TOKEN + PRIVATE}})
                self.assertEqual(self.run_probe(replies)[0], 1)

    def test_observable_version_or_permissions_changes_fail(self):
        for text in (signpost(hash_prefix="fedcba987654"), signpost(version=2),
                     signpost().replace("读：项目/Studiolo", "读：无")):
            replies = happy_replies()
            replies[3] = reply(tool(3, text))
            self.assertEqual(self.run_probe(replies)[0], 1)
        replies = happy_replies()
        replies[4]["body"] = replies[4]["body"].replace(HASH.encode(), b"fedcba987654")
        self.assertEqual(self.run_probe(replies)[0], 1)

    def test_six_tools_alone_and_tool_errors_are_insufficient(self):
        replies = happy_replies()
        change_result(replies, 2, lambda value: value["tools"].pop())
        self.assertEqual(self.run_probe(replies)[0], 1)
        for index in (3, 4):
            replies = happy_replies()
            change_result(replies, index, lambda value: value.update(isError=True))
            self.assertEqual(self.run_probe(replies)[0], 1)
        replies = happy_replies()
        replies[4] = reply(tool(4, "# 策略版本 invalid\n" + PRIVATE))
        self.assertEqual(self.run_probe(replies)[0], 1)

    def test_forbidden_path_requires_exact_allowlist_denial(self):
        for value in (tool(5, PRIVATE), {"jsonrpc": "2.0", "id": 5,
                                       "error": {"code": -32602, "message": PRIVATE}}):
            replies = happy_replies()
            replies[5] = reply(value)
            self.assertEqual(self.run_probe(replies)[0], 1)

    def test_invalid_token_fails_before_network(self):
        for token in ("", "x" * (probe.MAX_TOKEN_BYTES + 1), TOKEN + "\n" + PRIVATE):
            code, requests, _ = self.run_probe(happy_replies(), token=token)
            self.assertEqual(code, 1)
            self.assertEqual(requests, [])


if __name__ == "__main__":
    unittest.main()
