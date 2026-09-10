#!/usr/bin/env python3
"""副本上线探针的模拟 HTTP 回归；不连接真实服务或读取真实凭据。"""
import contextlib
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

sys.dont_write_bytecode = True


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


probe = load("replica_probe", "check-agent-replica.py")
fixtures = load("agent_fixtures", "test-check-agent.py")
OBSERVED = int(time.time() * 1000) - 500_000


def fresh(expected="ready", **changes):
    fields = {"status": expected, "stale": str(expected == "stale").lower(), "generation": "a" * 32,
              "source_digest": "b" * 64, "source_observed_at_ms": OBSERVED,
              "received_at_ms": OBSERVED + 100, "age_secs": 500 if expected == "stale" else 1,
              "max_age_secs": 300}
    fields.update(changes)
    return "副本状态 " + fields.pop("status") + "；" + "；".join(f"{key}={value}" for key, value in fields.items())


def replies(expected="ready"):
    line = fresh(expected)
    first = fixtures.result(1, {"protocolVersion": probe.CHECK.PROTOCOL,
                               "serverInfo": {"name": "studiolo-agent"},
                               "instructions": "# Studiolo 路牌\n" + line + "\n" + fixtures.PRIVATE_TEXT})
    first[1]["Mcp-Session-Id"] = fixtures.SESSION
    return [first, fixtures.response(202, raw=""),
            fixtures.result(3, {"tools": [{"name": name} for name in sorted(probe.CHECK.EXPECTED_TOOLS)]}),
            fixtures.tool_result(4, "# Studiolo 路牌\n" + line + "\n" + fixtures.PRIVATE_TEXT),
            fixtures.tool_result(5, "revision abc\n" + line + "\n# Studiolo · 总览\n" + fixtures.PRIVATE_TEXT),
            fixtures.tool_result(6, line + f"\n# 代次 1 · 1 条\nnote\t{probe.CHECK.INDEX_PATH}\t123\n"),
            fixtures.tool_result(7, line + f"\n# 代次 1 · 1 条\n{probe.CHECK.INDEX_PATH}:1\t" + fixtures.PRIVATE_TEXT)]


class ReplicaProbeTests(unittest.TestCase):
    def run_probe(self, responses, expected="ready", url=fixtures.BASE):
        transport = fixtures.FakeHTTP(responses)
        original_build = urllib.request.build_opener
        output = io.StringIO()
        with mock.patch.object(probe.urllib.request, "build_opener",
                               side_effect=lambda *handlers: original_build(*handlers, transport)), \
                mock.patch("builtins.open", mock.mock_open(read_data=fixtures.TOKEN)) as token_open, \
                contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            code = probe.main([url, "/fixture-token-file", "--expected", expected])
        for secret in (fixtures.TOKEN, fixtures.PRIVATE_TEXT, fixtures.SESSION):
            self.assertNotIn(secret, output.getvalue())
        token_open.assert_called_once_with("/fixture-token-file", encoding="utf-8")
        return code, output.getvalue(), transport.requests

    def test_ready_and_stale_json_sse_report_only_validated_receipt(self):
        for expected in ("ready", "stale"):
            for sse in (False, True):
                with self.subTest(expected=expected, sse=sse):
                    batch = replies(expected)
                    if sse:
                        for index, (status, headers, body) in enumerate(batch):
                            if status == 200:
                                headers.replace_header("Content-Type", "text/event-stream")
                                batch[index] = (status, headers, "data: " + body + "\n\n")
                    code, output, requests = self.run_probe(batch, expected)
                    self.assertEqual(code, 0)
                    summary = json.loads(output)
                    self.assertEqual(summary["status"], expected)
                    self.assertEqual(summary["source_observed_at_ms"], OBSERVED)
                    self.assertEqual(summary["responses_checked"], 5)
                    self.assertEqual(len(requests), 7)
                    self.assertTrue(all(request.get_header("Authorization") == "Bearer " + fixtures.TOKEN for request in requests))
                    self.assertTrue(all(request.get_header("Mcp-session-id") == fixtures.SESSION for request in requests[1:]))

    def test_any_tool_receipt_or_expected_status_mismatch_fails(self):
        for index in range(3, 7):
            for changes in ({"generation": "c" * 32}, {"source_digest": "d" * 64},
                            {"source_observed_at_ms": OBSERVED - 1}, {"received_at_ms": OBSERVED + 101},
                            {"max_age_secs": 301}):
                with self.subTest(index=index, changes=changes):
                    batch = replies()
                    status, headers, body = batch[index]
                    batch[index] = (status, headers, body.replace(fresh(), fresh(**changes)))
                    self.assertEqual(self.run_probe(batch)[0], 1)
        self.assertEqual(self.run_probe(replies("stale"))[0], 1)

    def test_elapsed_age_can_advance_without_changing_receipt(self):
        batch = replies()
        status, headers, body = batch[-1]
        batch[-1] = status, headers, body.replace("age_secs=1", "age_secs=2")
        code, output, _ = self.run_probe(batch)
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(output)["age_secs_max"], 2)

    def test_invalid_protocol_metadata_cannot_be_replaced_by_body_text(self):
        for text in (fresh(stale="true"), fresh(generation="bad"), fresh(source_digest="BAD"),
                     fresh(source_observed_at_ms=0), fresh(received_at_ms=OBSERVED - 1),
                     fresh(received_at_ms=int(time.time() * 1000) + 60_000),
                     fresh(age_secs=301), fresh(max_age_secs=0), fresh(status="sync_failed"),
                     fixtures.PRIVATE_TEXT + "\n" + fresh(), fresh() + "；extra=" + fixtures.TOKEN):
            with self.subTest(text_kind=text[:20]):
                batch = replies()
                status, headers, body = batch[4]
                value = json.loads(body)
                value["result"]["content"][0]["text"] = "revision abc\n" + text + "\n# Studiolo · 总览"
                batch[4] = status, headers, json.dumps(value)
                self.assertEqual(self.run_probe(batch)[0], 1)

    def test_http_rpc_tool_errors_and_invalid_json_are_redacted(self):
        for index in range(7):
            for failure in ("http", "rpc", "json", "tool"):
                if index == 1 and failure != "http" or index < 3 and failure == "tool":
                    continue
                with self.subTest(index=index, failure=failure):
                    batch = replies()
                    status, headers, body = batch[index]
                    secret = fixtures.PRIVATE_TEXT + fixtures.TOKEN + fixtures.SESSION
                    if failure == "http":
                        batch[index] = fixtures.response(502, raw=secret)
                    elif failure == "json":
                        batch[index] = 200, headers, secret
                    else:
                        value = json.loads(body)
                        if failure == "rpc":
                            value.pop("result")
                            value["error"] = {"code": -32603, "message": secret}
                        else:
                            value["result"]["isError"] = True
                        batch[index] = 200, headers, json.dumps(value)
                    self.assertEqual(self.run_probe(batch)[0], 1)

    def test_redirects_and_transport_errors_do_not_leak_secrets(self):
        for code in (301, 302, 303, 307, 308):
            batch = [fixtures.response(code, headers={"Location": "http://other.example/stolen"})] + replies()
            result, _, requests = self.run_probe(batch)
            self.assertEqual(result, 1)
            self.assertEqual(len(requests), 1)
        for error in (urllib.error.URLError(fixtures.TOKEN), probe.http.client.BadStatusLine(fixtures.PRIVATE_TEXT)):
            self.assertEqual(self.run_probe([error])[0], 1)

    def test_missing_identity_session_or_tools_fails(self):
        for mutation in ("session", "identity", "protocol", "tools"):
            batch = replies()
            if mutation == "session":
                del batch[0][1]["Mcp-Session-Id"]
            else:
                index = 2 if mutation == "tools" else 0
                status, headers, body = batch[index]
                value = json.loads(body)
                if mutation == "tools":
                    value["result"]["tools"] = []
                elif mutation == "identity":
                    value["result"]["serverInfo"]["name"] = "other"
                else:
                    value["result"]["protocolVersion"] = "other"
                batch[index] = status, headers, json.dumps(value)
            self.assertEqual(self.run_probe(batch)[0], 1)

    def test_url_rejects_embedded_credentials_before_any_request(self):
        for url in ("http://user:secret@example/mcp", "http://example/mcp?token=secret", "file:///tmp/mcp"):
            code, _, requests = self.run_probe([], url=url)
            self.assertEqual(code, 1)
            self.assertEqual(requests, [])


if __name__ == "__main__":
    unittest.main()
