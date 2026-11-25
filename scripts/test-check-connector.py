#!/usr/bin/env python3
"""check-connector.py 回归：本机回环上的假服务和内存传输，不联网，不使用真实凭据。"""
import contextlib
from email.message import Message
import importlib.util
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock
import urllib.error
import urllib.request
import urllib.response

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("check_connector", Path(__file__).with_name("check-connector.py"))
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)

TOKEN, PRIVATE = "fixture-token-never-print", "fixture-title-never-print"
LOCAL, REMOTE = "http://127.0.0.1:4400", "https://agent.example"
PROJECT = "项目/Studiolo-Docs"
TAG = '"' + "a" * 32 + '"'


def reply(status, value=None, headers=None, marked=True, raw=None):
    metadata = Message()
    metadata["Content-Type"] = "application/json"
    metadata["Cache-Control"] = "no-store"
    if marked:
        metadata["Studiolo-Connect"] = "1"
    for name, item in (headers or {}).items():
        del metadata[name]
        if item is not None:
            metadata[name] = item
    return status, metadata, raw if raw is not None else json.dumps(value, ensure_ascii=False).encode()


def listing(*names, **changes):
    project = {"project": PROJECT, "items": [{"title": PRIVATE}], "finished": 2, "truncated": False}
    project.update(changes)
    projects = [dict(project, project=name) for name in names or (PROJECT,)]
    return reply(200, {"generated_at": "2026-11-24T00:00:00Z", "projects": projects}, {"ETag": TAG})


def refused(status, headers=None):
    return reply(status, {"error": PRIVATE}, headers)


def challenge(status=403):
    return reply(status, raw=PRIVATE.encode(), marked=False, headers={"cf-mitigated": "challenge"})


def handshake(text=probe.POLICY_DENIED_PREFIX + "这个类型是连接器"):
    return reply(200, {"jsonrpc": "2.0", "id": 1, "result": {"instructions": text}}, marked=False)


def unmarked(status):
    return reply(status, {"error": PRIVATE}, marked=False)


def board_replies():
    return [listing(), reply(304, raw=b"", headers={"ETag": TAG}), refused(403)]


def rest():
    return [unmarked(403), handshake()]


def status_replies():
    return [refused(422), refused(403)]


class FakeHTTP(urllib.request.HTTPSHandler, urllib.request.HTTPHandler):
    """两种协议都由它答：少继承一个，build_opener 就会补上真的那个去连网。"""

    def __init__(self, batch):
        super().__init__()
        self.batch, self.requests = iter(batch), []

    def http_open(self, request):
        self.requests.append(request)
        value = next(self.batch)
        if isinstance(value, Exception):
            raise value
        status, headers, body = value
        stream = io.BytesIO(body)
        if not 200 <= status < 300:
            stream.read = mock.Mock(side_effect=AssertionError("不得读取拒绝正文"))
        answer = urllib.response.addinfourl(stream, headers, request.full_url, status)
        answer.msg = "fixture"
        return answer

    https_open = http_open


class CheckConnector(unittest.TestCase):
    def run_main(self, batch, *args, base=LOCAL, token=TOKEN):
        transport = FakeHTTP(batch)
        opener = urllib.request.build_opener(transport, probe.NoRedirect())
        out, err = io.StringIO(), io.StringIO()
        with tempfile.NamedTemporaryFile("w", suffix=".token") as file:
            file.write(token + "\n")
            file.flush()
            with mock.patch.object(probe.urllib.request, "build_opener", return_value=opener), \
                    contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                code = probe.main([base, file.name, *args])
        printed = out.getvalue() + err.getvalue()
        for secret in (TOKEN, PRIVATE):
            self.assertNotIn(secret, printed)
        return code, printed, transport.requests

    def fails(self, batch, *args, says, **kwargs):
        code, printed, _ = self.run_main(batch, *args, **kwargs)
        self.assertEqual(code, 1, printed)
        self.assertIn(says, printed)
        self.assertNotIn("被边缘拦了", printed)

    def test_board_passes_on_loopback_and_also_checks_mcp_and_read(self):
        code, printed, requests = self.run_main(board_replies() + rest(), "board", "--project", PROJECT)
        self.assertEqual(code, 0, printed)
        self.assertEqual([(request.get_method(), request.selector) for request in requests],
                         [("GET", "/connect/v1/board"), ("GET", "/connect/v1/board"),
                          ("PUT", "/connect/v1/sessions"), ("GET", "/read/handshake"), ("POST", "/mcp")])
        for request in requests:
            self.assertEqual(request.get_header("User-agent"), probe.USER_AGENT)
            self.assertEqual(request.get_header("Authorization"), "Bearer " + TOKEN)
        self.assertEqual(requests[1].get_header("If-none-match"), TAG)

    def test_remote_checks_only_the_connect_entry(self):
        code, printed, requests = self.run_main(board_replies(), "board", base=REMOTE)
        self.assertEqual((code, len(requests)), (0, 3), printed)
        self.assertIn("没核 MCP 和 /read", printed)

    def test_weak_etag_and_a_real_change_are_accepted(self):
        weak = "W/" + TAG
        first = reply(200, json.loads(listing()[2]), {"ETag": weak})
        code, printed, _ = self.run_main([first, reply(304, raw=b""), refused(403)], "board", base=REMOTE)
        self.assertEqual(code, 0, printed)
        changed = reply(200, json.loads(listing()[2]), {"ETag": '"' + "b" * 32 + '"'})
        code, printed, _ = self.run_main([listing(), changed, refused(403)], "board", base=REMOTE)
        self.assertEqual(code, 0, printed)

    def test_status_never_sends_a_snapshot_that_could_be_accepted(self):
        code, printed, requests = self.run_main(status_replies() + rest(), "status")
        self.assertEqual(code, 0, printed)
        body = json.loads(requests[0].data)
        self.assertFalse(set(body) <= {"seq", "sessions"}, "多一个字段才会整份不收")

    def test_a_challenge_is_reported_as_the_edge_wherever_it_shows_up(self):
        board, status = board_replies(), status_replies()
        for batch, entry in (([challenge()], "board"), ([challenge(503)], "status"),
                             ([board[0], challenge()], "board"), (board[:2] + [challenge()], "board"),
                             ([status[0], challenge()], "status"), (board + [challenge()], "board"),
                             (board + [unmarked(403), challenge()], "board")):
            code, printed, _ = self.run_main(batch, entry)
            self.assertEqual(code, 3, printed)
            self.assertIn("被边缘拦了", printed)
            self.assertNotIn("核对失败", printed)

    def test_an_answer_without_the_marker_is_the_edge_whatever_its_status(self):
        for status in (200, 401, 403, 404, 429, 502, 503):
            code, printed, _ = self.run_main([unmarked(status)], "board")
            self.assertEqual(code, 3, printed)
            self.assertIn("没有 Studiolo-Connect 头", printed)
        code, printed, _ = self.run_main([reply(200, {}, {"Studiolo-Connect": "yes"})], "board")
        self.assertEqual(code, 3, printed)

    def test_wrong_token_or_missing_permission_is_not_called_the_edge(self):
        self.fails([refused(401)], "board", says="HTTP 200")
        self.fails([refused(403)], "board", says="HTTP 200")
        self.fails([refused(403)], "status", says="HTTP 422")
        self.fails([refused(503)], "status", says="HTTP 422")

    def test_a_token_that_can_do_more_than_one_thing_fails(self):
        self.fails(board_replies()[:2] + [reply(200, {"accepted": 0})], "board", says="HTTP 403")
        self.fails([refused(422), listing()], "status", says="HTTP 403")
        self.fails(board_replies() + [reply(200, raw=PRIVATE.encode(), marked=False)], "board", says="/read/handshake")
        self.fails(board_replies() + [unmarked(403), handshake("# Studiolo 路牌\n" + PRIVATE)], "board", says="MCP")
        self.fails(board_replies() + [unmarked(403), handshake(None)], "board", says="MCP")

    def test_roster_must_match_exactly_when_given(self):
        self.fails([listing(PROJECT, "项目/Other-Docs")], "board", "--project", PROJECT, says="少 0 个、多 1 个")
        self.fails([listing("项目/Other-Docs")], "board", "--project", PROJECT, says="少 1 个、多 1 个")
        self.fails([listing(PROJECT, PROJECT)], "board", says="两次")

    def test_contract_shape_of_the_list(self):
        for batch in ([listing(extra=1)], [listing(finished="2")], [listing(truncated=0)], [listing(items=None)],
                      [reply(200, {"projects": []}, {"ETag": TAG})], [reply(200, raw=b"[]", headers={"ETag": TAG})],
                      [reply(200, raw=b"not json", headers={"ETag": TAG})]):
            self.fails(batch, "board", says="列表")
        self.fails([reply(200, json.loads(listing()[2]))], "board", says="ETag")
        self.fails([reply(200, json.loads(listing()[2]), {"ETag": TAG, "Cache-Control": None})],
                   "board", says="no-store")
        self.fails([listing(), listing()], "board", says="304")

    def test_oversized_answer_fails(self):
        with mock.patch.object(probe, "MAX_BODY", 8):
            self.fails([listing()], "board", says="上限")

    def test_refused_credentials(self):
        for mode in ("--no-token", "--bad-token"):
            batch = [refused(401, {"WWW-Authenticate": "Bearer realm=x"})] * 2
            code, printed, requests = self.run_main(batch, "board", mode)
            self.assertEqual(code, 0, printed)
            for request in requests:
                self.assertNotIn(TOKEN, request.get_header("Authorization") or "")
        self.fails([refused(401)], "board", "--no-token", says="Bearer challenge")
        self.fails([refused(403, {"WWW-Authenticate": "Bearer"})], "board", "--bad-token", says="401")
        code, _, _ = self.run_main([challenge()], "board", "--bad-token")
        self.assertEqual(code, 3)

    def test_token_never_leaves_over_plain_http_or_to_another_path(self):
        for base in ("http://agent.example", "ftp://127.0.0.1", "https://agent.example/mcp", "https://u:p@agent.example",
                     "https://agent.example?x=1", "http://127.0.0.1.example", "agent.example"):
            self.fails([], "board", says="服务地址", base=base)

    def test_redirects_are_not_followed_and_network_errors_stay_quiet(self):
        code, _, requests = self.run_main([refused(302, {"Location": "https://elsewhere.example/"})], "board")
        self.assertEqual((code, len(requests)), (1, 1))
        self.fails([urllib.error.URLError(PRIVATE)], "board", says="连不上")
        self.fails([], "board", says="token 文件", token="")


if __name__ == "__main__":
    unittest.main()
