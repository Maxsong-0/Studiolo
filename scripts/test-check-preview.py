#!/usr/bin/env python3
"""预览冒烟检查回归：模拟 HTTP 响应，不联网、不读取真实笔记。"""

import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import sys
import unittest
from unittest import mock
from urllib.parse import parse_qs, urlsplit
from typing import NamedTuple

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("check_preview", Path(__file__).with_name("check-preview.py"))
preview = importlib.util.module_from_spec(spec)
spec.loader.exec_module(preview)


def entry(path, kind, children=None):
    return {"path": path, "name": path.rsplit("/", 1)[-1], "kind": kind,
            "size_bytes": None if kind == "directory" else "24", "child_count": children}


PAGES = {
    "": [entry("附件演示", "directory", 2), entry("项目", "directory", 4),
         entry("README.md", "markdown")],
    "附件演示": [entry(preview.ATTACHMENTS, "directory", 3),
                 entry(preview.EMPTY_DIRECTORY, "directory", 0)],
    preview.EMPTY_DIRECTORY: [],
    preview.ATTACHMENTS: [entry(preview.ATTACHMENTS + "/" + name, "attachment")
                          for name in sorted(preview.ATTACHMENT_NAMES)],
}


class RawResponse(NamedTuple):
    mime: str
    body: bytes


HTML = b'''<script src="/assets/app.js"></script>
<link rel="stylesheet" href="/assets/app.css">
<link rel="icon" type="image/svg+xml" href="/assets/favicon.svg">'''
ASSETS = {
    "/assets/app.js": RawResponse("text/javascript; charset=utf-8", b"const fixture = true;"),
    "/assets/app.css": RawResponse("text/css; charset=utf-8", b"body { color: black; }"),
    "/assets/favicon.svg": RawResponse("image/svg+xml", b'<svg xmlns="http://www.w3.org/2000/svg"/>'),
}


class Transport:
    def __init__(self, change=None):
        self.requests = []
        self.change = change

    def reply(self, method, url, headers):
        self.requests.append((method, url, headers))
        location = urlsplit(url)
        query = parse_qs(location.query, keep_blank_values=True)
        status, value = 404, {}
        if method != "GET":
            status = 405
        elif headers.get("Origin"):
            status = 403
        elif location.path in ASSETS:
            status, value = 200, ASSETS[location.path]
        elif location.path == "/api/v1/bootstrap":
            status, value = 200, {"authentication": "local"}
        elif location.path == "/api/tree":
            path = query["path"][0]
            generation = int(query["generation"][0])
            offset = int(query["offset"][0])
            if generation != 7:
                status = 409
            elif path.startswith("../"):
                status = 400
            elif path in PAGES:
                items = copy.deepcopy(PAGES[path])
                if offset and offset >= len(items):
                    status = 400
                else:
                    status, value = 200, {
                        "generation": generation, "path": path, "offset": offset,
                        "total": len(items), "entries": items, "next_offset": None,
                    }
        if self.change:
            status, value = self.change(method, url, status, value)
        return status, value

    def connection(self, host, port, timeout):
        assert (host, port, timeout) == ("127.0.0.1", 4322, 10)
        transport = self

        class Connection:
            def request(self, method, url, headers):
                self.status, value = transport.reply(method, url, headers)
                self.mime, self.body = value if isinstance(value, RawResponse) else (
                    "application/json", json.dumps(value).encode())

            def getresponse(self):
                return self

            def read(self, limit):
                return self.body[:limit]

            def getheader(self, name, default):
                return self.mime

            def close(self):
                pass

        return Connection()


class PreviewTests(unittest.TestCase):
    def run_check(self, operation, change=None):
        transport = Transport(change)
        checks = preview.Checks(4322)
        output = io.StringIO()
        with mock.patch.object(preview.http.client, "HTTPConnection", transport.connection), \
                contextlib.redirect_stdout(output):
            try:
                operation(checks)
            except preview.CheckFailed:
                pass
        self.assertNotIn("private-content-never-print", output.getvalue())
        return checks, transport.requests

    def run_operation(self, value):
        """直接调用 Checks.run，返回计数、是否抛出 CheckFailed 以及标准输出。"""
        checks = preview.Checks(4322)
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            try:
                checks.run("显式结果", lambda: value)
            except preview.CheckFailed:
                return checks, True, output.getvalue()
        return checks, False, output.getvalue()

    def test_run_counts_only_explicit_true_as_pass(self):
        # None、0、空串、False 以及非 True 的真值（含请求响应本身）都不能被当成通过。
        for value in (None, 0, "", False, 1, "通过", RawResponse("text/html", b"<html>"),
                      {"authentication": "local"}):
            with self.subTest(value=value):
                checks, raised, output = self.run_operation(value)
                self.assertTrue(raised)
                self.assertEqual((checks.total, checks.failed), (1, 1))
                self.assertEqual(output, "显式结果：失败\n")
        checks, raised, output = self.run_operation(True)
        self.assertFalse(raised)
        self.assertEqual((checks.total, checks.failed), (1, 0))
        self.assertEqual(output, "显式结果：通过\n")

    def test_fetch_hands_back_response_only_after_explicit_pass(self):
        responses = []
        checks, _ = self.run_check(lambda checks: responses.append(
            checks.fetch("前端认证引导", preview.api("v1/bootstrap"), as_json=True)))
        self.assertEqual((checks.total, checks.failed), (1, 0))
        self.assertEqual(responses, [{"authentication": "local"}])
        checks, _ = self.run_check(lambda checks: responses.append(
            checks.fetch("应用会话未开放", preview.api("v1/session"))))
        self.assertEqual((checks.total, checks.failed), (1, 1))
        self.assertEqual(len(responses), 1)

    def test_static_entry_fetches_svg_icon_and_checks_mime(self):
        checks, requests = self.run_check(lambda checks: preview.check_assets(checks, HTML))
        self.assertEqual(checks.failed, 0)
        self.assertEqual({url for _, url, _ in requests}, set(ASSETS))

    def test_missing_external_and_non_svg_icon_fail_before_fetch(self):
        for html in [HTML.split(b'<link rel="icon"')[0],
                     HTML.replace(b"/assets/favicon.svg", b"https://external.example/icon.svg"),
                     HTML.replace(b"/assets/favicon.svg", b"data:image/svg+xml,fixture"),
                     HTML.replace(b"/assets/favicon.svg", b"/assets/app.js")]:
            with self.subTest(html=html):
                checks, requests = self.run_check(lambda checks: preview.check_assets(checks, html))
                self.assertEqual(checks.failed, 1)
                self.assertEqual(requests, [])

    def test_svg_404_wrong_mime_and_empty_body_are_not_accepted(self):
        for status, value in [(404, {}), (200, RawResponse("text/html", b"private-content-never-print")),
                              (200, RawResponse("image/svg+xml", b""))]:
            def change(method, url, original_status, original_value):
                return (status, value) if url.endswith(".svg") else (original_status, original_value)

            with self.subTest(status=status, value=value):
                checks, _ = self.run_check(lambda checks: preview.check_assets(checks, HTML), change)
                self.assertEqual(checks.failed, 1)

    def test_local_bootstrap_and_disabled_session_routes(self):
        checks, requests = self.run_check(preview.check_bootstrap)
        self.assertEqual(checks.failed, 0)
        self.assertEqual(checks.total, 4)
        self.assertEqual([item[:2] for item in requests], [
            ("GET", "/api/v1/bootstrap"), ("GET", "/api/v1/session"),
            ("POST", "/api/v1/sessions"),
        ])

    def test_old_or_nonlocal_bootstrap_stops_before_content(self):
        # 假值正文（null、0、空串、false、空数组）同样不能让引导检查通过。
        for status, value in [(404, {}), (401, {}), (200, {"authentication": "session"}),
                              (200, {"authentication": "access"}),
                              (200, {"authentication": "local", "extra": True}),
                              (200, None), (200, 0), (200, ""), (200, False), (200, [])]:
            with self.subTest(status=status, value=value):
                checks, requests = self.run_check(
                    preview.check_bootstrap, lambda *args: (status, value))
                self.assertEqual(checks.failed, 1)
                self.assertEqual(len(requests), 1)

    def test_tree_directory_attachment_and_denial_chain(self):
        checks, requests = self.run_check(lambda checks: preview.check_tree(checks, 7))
        self.assertEqual(checks.failed, 0)
        self.assertTrue(any("generation=6" in url for _, url, _ in requests))
        self.assertTrue(any(method == "POST" for method, _, _ in requests))
        self.assertTrue(any(headers.get("Origin") for _, _, headers in requests))
        self.assertFalse(any("refresh" in url for _, url, _ in requests))

    def test_tree_404_and_stale_generation_are_not_accepted(self):
        for status in (404, 409):
            with self.subTest(status=status):
                checks, requests = self.run_check(
                    lambda checks: preview.check_tree(checks, 7), lambda *args: (status, {}))
                self.assertEqual(checks.failed, 1)
                self.assertEqual(len(requests), 1)

    def test_malformed_tree_and_embedded_content_fail_without_printing_body(self):
        for field, value in [("generation", 6), ("path", "项目"), ("offset", 1),
                             ("total", 99), ("next_offset", 200),
                             ("content", "private-content-never-print")]:
            def change(method, url, status, page):
                page[field] = value
                return status, page

            with self.subTest(field=field):
                checks, requests = self.run_check(
                    lambda checks: preview.check_tree(checks, 7), change)
                self.assertEqual(checks.failed, 1)
                self.assertEqual(len(requests), 1)

    def test_attachment_body_or_hidden_entry_is_not_accepted(self):
        for field, value in [("content", "private-content-never-print"), ("name", ".hidden"),
                             ("size_bytes", "18446744073709551616")]:
            def change(method, url, status, page):
                if page.get("path") == preview.ATTACHMENTS:
                    page["entries"][0][field] = value
                return status, page

            with self.subTest(field=field):
                checks, _ = self.run_check(lambda checks: preview.check_tree(checks, 7), change)
                self.assertEqual(checks.failed, 1)

    def test_broken_tree_denials_fail_instead_of_reporting_pass(self):
        for denial in (400, 403, 405, 409):
            with self.subTest(denial=denial):
                checks, _ = self.run_check(
                    lambda checks: preview.check_tree(checks, 7),
                    lambda method, url, status, value: (200 if status == denial else status, value))
                self.assertEqual(checks.failed, 1)


if __name__ == "__main__":
    unittest.main()
