#!/usr/bin/env python3
"""HTTP 发布探针的内存传输回归，不连接真实服务或使用真实凭据。"""
import contextlib
from email.message import Message
import importlib.util
import io
import json
from pathlib import Path
import sys
import time
import unittest
from unittest import mock
import urllib.error
import urllib.request
import urllib.response

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location("http_probe", Path(__file__).with_name("check-agent-http.py"))
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)
TOKEN, PRIVATE = "fixture-token-never-print", "fixture-note-never-print"
SESSION, INSTANCE = "01K4ABCD000000000000000001", "01K4ABCD000000000000000002"
ORIGIN = "http://agent.example"
NOW = int(time.time() * 1000)


def fresh(**changes):
    fields = {"generation": "a" * 32, "source_digest": "b" * 64,
              "source_observed_at_ms": NOW - 1000, "received_at_ms": NOW - 900,
              "age_secs": 1, "max_age_secs": 300}
    fields.update(changes)
    return "副本状态 ready；stale=false；" + "；".join(f"{key}={value}" for key, value in fields.items())


def response(status=200, text="", headers=None):
    metadata = Message()
    for name, value in (("Content-Type", "text/plain; charset=utf-8"), ("Cache-Control", "no-store"),
                        ("Pragma", "no-cache"), ("X-Content-Type-Options", "nosniff"),
                        ("Referrer-Policy", "no-referrer")):
        metadata[name] = value
    if status == 200:
        metadata["Studiolo-Session-Id"], metadata["Studiolo-Instance-Id"] = SESSION, INSTANCE
    if status == 401:
        metadata["WWW-Authenticate"] = 'Bearer realm="studiolo-agent"'
    for key, value in (headers or {}).items():
        metadata[key] = value
    return status, metadata, text


def replies(blank="\n"):
    return [response(text="# Studiolo 路牌（代次 1，今天）\n" + fresh() + "\n" + blank
                     + "## 1 我是谁\n类型 claude · 实例 " + INSTANCE + " · 注册 今天\n" + PRIVATE),
            response(text=fresh() + f"\n# 代次 1 · 1 条\nnote\t{probe.INDEX_PATH}\t123"),
            response(text="revision " + "c" * 64 + "\n" + fresh() + "\n# Studiolo · 总览\n" + PRIVATE),
            response(text=fresh() + f"\n# 代次 1 · 1 条\n{probe.INDEX_PATH}:1\t" + PRIVATE),
            *[response(401, PRIVATE + TOKEN + SESSION) for _ in range(11)],
            *[response(400, PRIVATE + TOKEN + SESSION) for _ in range(3)]]


class Body(io.BytesIO):
    def __init__(self, value, denial):
        super().__init__(value if isinstance(value, bytes) else value.encode())
        self.denial = denial

    def read(self, size=-1):
        if self.denial:
            raise AssertionError("不得读取拒绝正文")
        if size < 0 or size > probe.MAX_RESPONSE_BYTES + 1:
            raise AssertionError("必须有界读取")
        return super().read(size)


class FakeHTTP(urllib.request.HTTPHandler):
    def __init__(self, batch):
        super().__init__()
        self.batch, self.requests = iter(batch), []

    def http_open(self, request):
        self.requests.append(request)
        value = next(self.batch)
        if isinstance(value, Exception):
            raise value
        status, headers, text = value
        reply = urllib.response.addinfourl(Body(text, status != 200), headers,
                                         request.full_url, code=status)
        reply.msg = "fixture response"
        return reply


class HttpProbeTests(unittest.TestCase):
    def run_probe(self, batch, origin=ORIGIN, token=TOKEN):
        transport, output = FakeHTTP(batch), io.StringIO()
        build = urllib.request.build_opener
        handlers = []

        def opener(*configured):
            handlers.extend(configured)
            return build(*configured, transport)

        with mock.patch.object(probe.urllib.request, "build_opener", side_effect=opener), \
                mock.patch("builtins.open", mock.mock_open(read_data=token)), \
                mock.patch.object(probe.time, "time", return_value=NOW / 1000), \
                contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            code = probe.main([origin, "/fixture-token-file"])
        for secret in (TOKEN, PRIVATE, SESSION, INSTANCE):
            self.assertNotIn(secret, output.getvalue())
        return code, output.getvalue(), transport.requests, handlers

    def test_full_chain_has_fixed_summary_headers_timeouts_and_no_environment_proxy(self):
        for blank in ("", "\n"):
            code, output, requests, handlers = self.run_probe(replies(blank))
            self.assertEqual(code, 0)
            summary = json.loads(output)
            self.assertEqual((summary["responses_checked"], summary["denials_checked"]), (18, 14))
            self.assertEqual(summary["status"], "ready")
            self.assertEqual(summary["source_observed_at_ms"], NOW - 1000)
            self.assertEqual(len(requests), 18)
            self.assertTrue(any(isinstance(handler, urllib.request.ProxyHandler)
                                and handler.proxies == {} for handler in handlers))
            for request in requests:
                self.assertEqual(request.timeout, 15)
                self.assertEqual(request.get_method(), "GET")
                self.assertTrue(request.full_url.startswith(ORIGIN + "/read/"))
            self.assertIsNone(requests[0].get_header("Studiolo-session-id"))
            self.assertTrue(all(request.get_header("Studiolo-session-id") == SESSION
                                for request in requests[1:4]))
            self.assertEqual([r.get_header("Authorization") for r in requests[4:6]],
                             [None, "Bearer sag1_" + "0" * 64])
            self.assertTrue(all(r.get_header("Studiolo-session-id") is None for r in requests[12:15]))

    def test_receipt_change_is_the_only_retryable_failure(self):
        for index in range(1, 4):
            for changes in ({"generation": "c" * 32}, {"source_digest": "d" * 64},
                            {"received_at_ms": NOW - 800}, {"source_observed_at_ms": NOW - 1001},
                            {"max_age_secs": 301}):
                batch = replies()
                status, headers, text = batch[index]
                batch[index] = status, headers, text.replace(fresh(), fresh(**changes))
                self.assertEqual(self.run_probe(batch)[0], 2)
        batch = replies()
        status, headers, text = batch[3]
        batch[3] = status, headers, text.replace("age_secs=1", "age_secs=2")
        self.assertEqual(self.run_probe(batch)[0], 0)

    def test_body_freshness_forgery_invalid_status_age_and_time_are_rejected(self):
        for invalid in (PRIVATE + "\n" + fresh(), fresh() + "；extra=x", fresh(generation="bad"),
                        fresh().replace("ready", "sync_failed"), fresh().replace("stale=false", "stale=true"),
                        fresh(source_observed_at_ms=0), fresh(received_at_ms=NOW - 1001),
                        fresh(received_at_ms=NOW + 30_001), fresh(age_secs=301), fresh(max_age_secs=59),
                        fresh(source_observed_at_ms=NOW - 301_000, age_secs=1)):
            for index in range(4):
                batch = replies()
                status, headers, text = batch[index]
                batch[index] = status, headers, text.replace(fresh(), invalid)
                self.assertEqual(self.run_probe(batch)[0], 1)

    def test_all_required_headers_are_unique_and_checked_on_success_and_denial(self):
        for index in (0, 1, 2, 3, 4, 15):
            for name in ("Content-Type", "Cache-Control", "Pragma", "X-Content-Type-Options", "Referrer-Policy"):
                for mutation in ("missing", "duplicate", "wrong"):
                    batch = replies()
                    headers = batch[index][1]
                    if mutation == "missing":
                        del headers[name]
                    elif mutation == "duplicate":
                        headers[name] = headers[name]
                    else:
                        headers.replace_header(name, "invalid")
                    self.assertEqual(self.run_probe(batch)[0], 1)
        for encoding in ("gzip", "deflate"):
            batch = replies()
            batch[0][1]["Content-Encoding"] = encoding
            self.assertEqual(self.run_probe(batch)[0], 1)

    def test_missing_or_changed_session_identity_or_handshake_format_fails(self):
        for index in range(4):
            for name in ("Studiolo-Session-Id", "Studiolo-Instance-Id"):
                for value in (None, "bad", "01K4ABCD000000000000000003"):
                    batch = replies()
                    del batch[index][1][name]
                    if value is not None:
                        batch[index][1][name] = value
                    # 初次有效 session 可以任意，但实例必须与路牌匹配。
                    if index == 0 and name == "Studiolo-Session-Id" and value and value != "bad":
                        continue
                    self.assertEqual(self.run_probe(batch)[0], 1)
        for replacement in ("## 1 我是谁", "类型 claude", "# Studiolo 路牌"):
            batch = replies()
            status, headers, text = batch[0]
            batch[0] = status, headers, text.replace(replacement, "invalid")
            self.assertEqual(self.run_probe(batch)[0], 1)
        self.assertEqual(self.run_probe(replies("\n\n"))[0], 1)

    def test_false_content_contract_and_count_fail(self):
        for index, before, after in ((1, probe.INDEX_PATH, "other.md"), (1, "1 条", "2 条"),
                                     (2, "revision ", "version "), (2, "Studiolo · 总览", "other"),
                                     (3, probe.INDEX_PATH, "other.md"), (3, "1 条", "2 条")):
            batch = replies()
            status, headers, text = batch[index]
            batch[index] = status, headers, text.replace(before, after)
            self.assertEqual(self.run_probe(batch)[0], 1)

    def test_each_denial_requires_its_exact_status_and_authentication_challenge(self):
        for index in range(4, 18):
            for status in (200, 403, 502):
                batch = replies()
                batch[index] = response(status, PRIVATE + TOKEN + SESSION)
                self.assertEqual(self.run_probe(batch)[0], 1)
            batch = replies()
            batch[index][1]["Studiolo-Session-Id"] = SESSION
            self.assertEqual(self.run_probe(batch)[0], 1)
        for index in range(4, 15):
            batch = replies()
            del batch[index][1]["WWW-Authenticate"]
            self.assertEqual(self.run_probe(batch)[0], 1)

    def test_redirect_transport_and_invalid_utf8_are_safe_failures(self):
        for status in (301, 302, 303, 307, 308):
            code, _, requests, _ = self.run_probe([response(status, headers={"Location": ORIGIN + "/stolen"})])
            self.assertEqual((code, len(requests)), (1, 1))
        for error in (urllib.error.URLError(TOKEN), probe.http.client.BadStatusLine(PRIVATE)):
            self.assertEqual(self.run_probe([error])[0], 1)
        batch = replies()
        batch[0] = response(text=b"\xff")
        self.assertEqual(self.run_probe(batch)[0], 1)

    def test_bounded_reads_and_invalid_origin_or_token_fail_before_transport(self):
        with mock.patch.object(probe, "MAX_RESPONSE_BYTES", 32):
            self.assertEqual(self.run_probe(replies())[0], 1)
        for origin in ("http://user:secret@example", "http://@example", "http://example/path", "http://example?x=y",
                       "http://example/#fragment", "file:///tmp", "http://example:bad", "http://example\n"):
            code, _, requests, _ = self.run_probe([], origin=origin)
            self.assertEqual((code, requests), (1, []))
        for token in ("", "x" * 4097, "x\ny", "令牌", "x\x00y"):
            code, _, requests, _ = self.run_probe([], token=token)
            self.assertEqual((code, requests), (1, []))


if __name__ == "__main__":
    unittest.main()
