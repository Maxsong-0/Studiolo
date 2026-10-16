#!/usr/bin/env python3
"""check-agent.py 回归：模拟 urllib 的 HTTP 层，不联网，不使用真实凭据或 vault。"""
import contextlib
from email.message import Message
import importlib.util
import io
import json
from pathlib import Path
import sys
import unittest
from unittest import mock
import urllib.request
import urllib.response

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("check_agent", Path(__file__).with_name("check-agent.py"))
agent = importlib.util.module_from_spec(spec)
spec.loader.exec_module(agent)

TOKEN = "fixture-token-never-print"
PRIVATE_TEXT = "fixture-note-never-print"
SESSION = "fixture-session-never-print"
BASE = "http://agent.example/mcp"


def response(status, value=None, headers=None, raw=None):
    metadata = Message()
    metadata["Content-Type"] = "application/json"
    for key, item in (headers or {}).items():
        metadata[key] = item
    body = raw if raw is not None else json.dumps(value, ensure_ascii=False)
    return status, metadata, body


def result(request_id, value):
    return response(200, {"jsonrpc": "2.0", "id": request_id, "result": value})


def tool_result(request_id, text):
    return result(request_id, {"content": [{"type": "text", "text": text}]})


def happy_responses(policy="absent"):
    """policy="absent"：serve 未给 --policy（线上现状）；"attached"：已接入策略平面。"""
    mark = ("\n## 3 能力\n只读；" + agent.POLICY_ABSENT_MARK) if policy == "absent" else "\n## 3 能力\n读：…；策略版本 0123456789ab"
    initial = result(1, {
        "protocolVersion": agent.PROTOCOL, "serverInfo": {"name": "studiolo-agent"},
        "instructions": "# Studiolo 路牌\n" + PRIVATE_TEXT + mark,
    })
    initial[1]["mcp-session-id"] = SESSION
    probe = (response(200, {"jsonrpc": "2.0", "id": 8,
                            "error": {"code": -32600, "message": agent.POLICY_ABSENT_ERROR}})
             if policy == "absent" else tool_result(8, "# 策略版本 0123456789ab\n0 个 skill\n" + PRIVATE_TEXT))
    return [
        initial, response(202, raw=""),
        result(2, {"tools": [{"name": name} for name in sorted(agent.EXPECTED_TOOLS)]}),
        tool_result(3, "# Studiolo 路牌\n" + PRIVATE_TEXT),
        tool_result(4, "revision abc\n# Studiolo · 总览\n" + PRIVATE_TEXT),
        tool_result(5, f"note\t{agent.INDEX_PATH}\t123\n"),
        tool_result(6, f"{agent.INDEX_PATH}:1\t" + PRIVATE_TEXT),
        response(200, {"jsonrpc": "2.0", "id": 7,
                       "error": {"code": -32602, "message": "路径不在供给白名单内"}}),
        probe,
    ]


class FakeHTTP(urllib.request.HTTPHandler):
    def __init__(self, responses):
        super().__init__()
        self.responses = iter(responses)
        self.requests = []

    def http_open(self, request):
        self.requests.append(request)
        reply = next(self.responses)
        if isinstance(reply, Exception):
            raise reply
        status, headers, body = reply
        reply = urllib.response.addinfourl(io.BytesIO(body.encode()), headers,
                                         request.full_url, code=status)
        reply.msg = "fixture HTTP response"
        return reply


class CheckAgentTests(unittest.TestCase):
    def run_check(self, responses, mode=None):
        transport = FakeHTTP(responses)
        original_build = urllib.request.build_opener
        output = io.StringIO()
        args = [BASE, "/not-a-real-token-file"] + ([mode] if mode else [])
        with mock.patch.object(agent.urllib.request, "build_opener",
                               side_effect=lambda *handlers: original_build(*handlers, transport)), \
                mock.patch("builtins.open", mock.mock_open(read_data=TOKEN)) as token_open, \
                contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            code = agent.main(args)
        self.assertNotIn(TOKEN, output.getvalue())
        self.assertNotIn(PRIVATE_TEXT, output.getvalue())
        self.assertNotIn(SESSION, output.getvalue())
        if mode:
            token_open.assert_not_called()
        return code, transport.requests

    def test_complete_chain_passes_json_and_sse_without_exposing_content(self):
        for sse in (False, True):
            with self.subTest(sse=sse):
                responses = happy_responses()
                if sse:
                    for index, (status, headers, body) in enumerate(responses):
                        if status == 200:
                            headers.replace_header("Content-Type", "text/event-stream")
                            unrelated = 'data: {"jsonrpc":"2.0","method":"notifications/ping"}\n\n'
                            data = json.dumps(json.loads(body), ensure_ascii=False, indent=2)
                            encoded = "\n".join("data: " + line for line in data.splitlines())
                            responses[index] = (status, headers, unrelated + encoded + "\n\n")
                code, requests = self.run_check(responses)
                self.assertEqual(code, 0)
                self.assertEqual(len(requests), 9)
                self.assertTrue(all(request.get_header("Authorization") == "Bearer " + TOKEN
                                    for request in requests))
                self.assertTrue(all(request.get_header("Mcp-session-id") == SESSION
                                    for request in requests[1:]))

    def test_negative_modes_require_401_and_bearer_challenge(self):
        for mode in ("--no-token", "--bad-token"):
            for status in (200, 401, 403, 502):
                with self.subTest(mode=mode, status=status):
                    reply = response(status, headers={"WWW-Authenticate": 'Bearer realm="studiolo-agent"'},
                                     raw=PRIVATE_TEXT + TOKEN)
                    code, requests = self.run_check([reply], mode)
                    self.assertEqual(code, 0 if status == 401 else 1)
                    self.assertEqual(len(requests), 1)
                    expected = None if mode == "--no-token" else "Bearer sag1_" + "0" * 64
                    self.assertEqual(requests[0].get_header("Authorization"), expected)
            code, _ = self.run_check([response(401, headers={"WWW-Authenticate": "Basic"})], mode)
            self.assertEqual(code, 1)

    def test_unexpected_http_and_rpc_failures_cannot_pass(self):
        for index in range(9):
            for status in (403, 502):
                with self.subTest(index=index, status=status):
                    replies = happy_responses()
                    replies[index] = response(status, raw=PRIVATE_TEXT + TOKEN)
                    self.assertEqual(self.run_check(replies)[0], 1)
            if index == 1:
                continue
            with self.subTest(index=index, rpc_error=True):
                replies = happy_responses()
                request_id = json.loads(replies[index][2])["id"]
                replies[index] = response(200, {"jsonrpc": "2.0", "id": request_id,
                                               "error": {"code": -32603, "message": PRIVATE_TEXT + TOKEN}})
                self.assertEqual(self.run_check(replies)[0], 1)

    def test_missing_tools_and_tool_errors_cannot_pass(self):
        replies = happy_responses()
        replies[2] = result(2, {"tools": [{"name": "handshake"}]})
        self.assertEqual(self.run_check(replies)[0], 1)
        for index in range(3, 7):
            with self.subTest(index=index):
                replies = happy_responses()
                value = json.loads(replies[index][2])
                value["result"]["isError"] = True
                replies[index] = response(200, value)
                self.assertEqual(self.run_check(replies)[0], 1)

    def test_allowed_outside_read_and_wrong_denial_cannot_pass(self):
        for reply in (tool_result(7, PRIVATE_TEXT),
                      response(200, {"jsonrpc": "2.0", "id": 7,
                                     "error": {"code": -32602, "message": PRIVATE_TEXT}})):
            replies = happy_responses()
            replies[7] = reply
            self.assertEqual(self.run_check(replies)[0], 1)

    def test_tool_inventory_must_be_exactly_the_eight_tools(self):
        self.assertEqual(agent.EXPECTED_TOOLS, agent.READ_TOOLS | agent.POLICY_TOOLS | agent.BOARD_TOOLS)
        self.assertEqual(len(agent.EXPECTED_TOOLS), 8)
        for names in (sorted(agent.READ_TOOLS),                       # M4 之前的二进制
                      sorted(agent.EXPECTED_TOOLS - {"list_proposals"}),  # T-182 之前的二进制（M5 的七个）
                      sorted(agent.EXPECTED_TOOLS) + ["write_note"],  # 多出一个未登记工具
                      sorted(agent.EXPECTED_TOOLS - {"propose_desired"}) + ["handshake"]):
            with self.subTest(names=names):
                replies = happy_responses()
                replies[2] = result(2, {"tools": [{"name": name} for name in names]})
                self.assertEqual(self.run_check(replies)[0], 1)

    def test_attached_policy_plane_must_serve_the_skills_index(self):
        code, requests = self.run_check(happy_responses("attached"))
        self.assertEqual(code, 0)
        body = json.loads(requests[-1].data)
        self.assertEqual(body["params"], {"name": "policy_read", "arguments": {"kind": "skills"}})
        called = [json.loads(request.data).get("params", {}).get("name") for request in requests]
        self.assertNotIn("propose_desired", called)
        self.assertNotIn("list_proposals", called)
        for reply in (response(200, {"jsonrpc": "2.0", "id": 8,
                                     "error": {"code": -32600, "message": agent.POLICY_ABSENT_ERROR}}),
                      tool_result(8, PRIVATE_TEXT)):
            replies = happy_responses("attached")
            replies[8] = reply
            self.assertEqual(self.run_check(replies)[0], 1)

    def test_absent_policy_plane_must_refuse_policy_read(self):
        for reply in (tool_result(8, "# 策略版本 0123456789ab\n" + PRIVATE_TEXT),
                      response(200, {"jsonrpc": "2.0", "id": 8,
                                     "error": {"code": -32603, "message": agent.POLICY_ABSENT_ERROR}}),
                      response(200, {"jsonrpc": "2.0", "id": 8,
                                     "error": {"code": -32600, "message": PRIVATE_TEXT}})):
            replies = happy_responses()
            replies[8] = reply
            self.assertEqual(self.run_check(replies)[0], 1)

    def test_policy_denied_handshake_fails_without_echoing_the_reason(self):
        replies = happy_responses("attached")
        value = json.loads(replies[0][2])
        value["result"]["instructions"] = agent.POLICY_DENIED_PREFIX + PRIVATE_TEXT
        replies[0] = (200, replies[0][1], json.dumps(value))
        self.assertEqual(self.run_check(replies)[0], 1)

    def test_missing_session_invalid_json_wrong_id_and_degraded_signpost_fail(self):
        for mutation in ("session", "json", "id", "degraded", "initialized"):
            with self.subTest(mutation=mutation):
                replies = happy_responses()
                if mutation == "session":
                    del replies[0][1]["mcp-session-id"]
                elif mutation == "json":
                    replies[0] = response(200, raw=PRIVATE_TEXT + TOKEN)
                elif mutation == "initialized":
                    replies[1] = response(200, raw="")
                else:
                    value = json.loads(replies[0][2])
                    if mutation == "id":
                        value["id"] = 999
                    else:
                        value["result"]["instructions"] = "审计不可用 " + PRIVATE_TEXT
                    replies[0] = (200, replies[0][1], json.dumps(value))
                self.assertEqual(self.run_check(replies)[0], 1)

    def test_redirects_never_forward_authorization(self):
        for status in (301, 302, 303, 307, 308):
            for location in (BASE + "/moved", "http://other.example/stolen"):
                with self.subTest(status=status, location=location):
                    replies = [response(status, headers={"Location": location},
                                        raw=PRIVATE_TEXT + TOKEN)] + happy_responses()
                    code, requests = self.run_check(replies)
                    self.assertEqual(code, 1)
                    self.assertEqual(len(requests), 1)
                    self.assertEqual(requests[0].full_url, BASE)

    def test_transport_errors_are_redacted(self):
        for error in (urllib.error.URLError(PRIVATE_TEXT + TOKEN),
                      agent.http.client.BadStatusLine(PRIVATE_TEXT + TOKEN)):
            with self.subTest(error_type=type(error).__name__):
                self.assertEqual(self.run_check([error])[0], 1)


if __name__ == "__main__":
    unittest.main()
