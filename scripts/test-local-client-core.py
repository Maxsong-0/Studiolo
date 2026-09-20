#!/usr/bin/env python3
"""Focused local transport and notification durability checks; no model calls."""
from concurrent.futures import ThreadPoolExecutor
import io
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import unittest
import urllib.error
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parent))
from local_client.mcp import (MAX_BYTES, PROTOCOL, McpClient, McpError,
                              _NoRedirect, _response_message)
from local_client.store import Store


class Response(io.BytesIO):
    def __init__(self, raw=b"", status=200, headers=None):
        super().__init__(raw)
        self.status = status
        self.headers = headers or {"Content-Type": "application/json"}


def rpc(message, result, headers=None):
    raw = json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}).encode()
    return Response(raw, headers=headers)


class FakeOpener:
    def __init__(self, handler=None):
        self.messages = []
        self.requests = []
        self.handler = handler
        self.initializations = 0

    def open(self, request, timeout):
        assert timeout == 20
        message = json.loads(request.data)
        self.messages.append(message)
        self.requests.append(request)
        if message["method"] == "initialize":
            self.initializations += 1
            return rpc(message, {"protocolVersion": PROTOCOL, "capabilities": {}},
                       {"Content-Type": "application/json", "Mcp-Session-Id": "session-" + str(self.initializations)})
        if message["method"] == "notifications/initialized":
            return Response(status=202)
        if self.handler:
            return self.handler(message)
        return rpc(message, {"tools": []})


class PrivateFixture(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="studiolo-local-core-")
        self.root = Path(self.temporary.name).resolve()
        self.addCleanup(self.temporary.cleanup)
        self.token = self.root / "studiolo.token"
        self.token.write_text("private-example-bearer")
        self.token.chmod(0o600)
        self.state = self.root / "session.json"

    def client(self, handler=None):
        client = McpClient("http://127.0.0.1:43117/mcp", self.token, self.state)
        opener = FakeOpener(handler)
        client._opener = opener
        return client, opener

    def assertCode(self, code, function, *args, **kwargs):
        with self.assertRaises(McpError) as caught:
            function(*args, **kwargs)
        self.assertEqual(code, caught.exception.code)
        self.assertEqual("Studiolo MCP: " + code, str(caught.exception))


class ResponseTests(unittest.TestCase):
    def test_sse_ignores_empty_and_notification_events_and_joins_data(self):
        raw = (b": ping\r\n\r\ndata:\r\n\r\nevent: message\r\n"
               b"data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}\r\n\r\n"
               b"data: {\"jsonrpc\":\"2.0\",\"id\":7,\r\n"
               b"data: \"result\":{\"ok\":true}}\r\n\r\n: final\r\n\r\n")
        self.assertEqual({"ok": True}, _response_message(raw, "text/event-stream; charset=utf-8", 7))

    def test_wrong_or_duplicate_id_is_rejected(self):
        for raw in [b'{"jsonrpc":"2.0","id":9,"result":{}}',
                    b'{"jsonrpc":"2.0","id":true,"result":{}}']:
            with self.assertRaisesRegex(McpError, "response_id_mismatch"):
                _response_message(raw, "application/json", 1)
        event = b'data: {"jsonrpc":"2.0","id":1,"result":{}}\n\n'
        with self.assertRaisesRegex(McpError, "response_id_mismatch"):
            _response_message(event + event, "text/event-stream", 1)

    def test_server_error_body_is_not_exposed(self):
        raw = b'{"jsonrpc":"2.0","id":1,"error":{"message":"SECRET"}}'
        with self.assertRaises(McpError) as caught:
            _response_message(raw, "application/json", 1)
        self.assertEqual("Studiolo MCP: rpc_error", str(caught.exception))

    def test_invalid_json_content_type_and_empty_sse_fail_closed(self):
        for raw, content_type in [(b"no", "application/json"),
                                  (b"{}", "text/html"),
                                  (b"data:\n\n", "text/event-stream")]:
            with self.assertRaises(McpError):
                _response_message(raw, content_type, 1)


class McpTests(PrivateFixture):
    def test_initialization_and_session_survive_client_restart(self):
        client, opener = self.client()
        self.assertEqual({"tools": []}, client.tools_list())
        first = opener.messages[0]
        self.assertEqual("studiolo-local", first["params"]["clientInfo"]["name"])
        self.assertEqual(PROTOCOL, first["params"]["protocolVersion"])
        second_client = McpClient(client.endpoint, self.token, self.state)
        second_client._opener = opener
        second_client.tools_list()
        self.assertEqual(1, opener.initializations)
        self.assertEqual([1, 2, 3], [item["id"] for item in opener.messages if "id" in item])
        self.assertEqual("session-1", opener.requests[-1].get_header("Mcp-session-id"))
        self.assertEqual(0o600, stat.S_IMODE(self.state.stat().st_mode))
        self.assertEqual(0o600, stat.S_IMODE(client.lock_file.stat().st_mode))
        self.assertNotIn(self.token.read_text(), self.state.read_text())

    def test_board_decodes_only_board_text_json(self):
        client, opener = self.client(lambda message: rpc(message, {
            "content": [{"type": "text", "text": '{"task_id":"T-42"}'}], "isError": False}))
        self.assertEqual({"task_id": "T-42"}, client.board("项目/Studiolo", "get", {"task_id": "T-42"}))
        self.assertEqual("board", opener.messages[-1]["params"]["name"])
        self.assertEqual("项目/Studiolo", opener.messages[-1]["params"]["arguments"]["project"])

    def test_tool_error_is_safe(self):
        client, _ = self.client(lambda message: rpc(message, {
            "content": [{"type": "text", "text": "SECRET"}], "isError": True}))
        self.assertCode("tool_error", client.board, "项目/Studiolo", "get", {})

    def test_lost_write_reply_is_never_retried(self):
        applied = []

        def commit_then_disconnect(message):
            applied.append(message)
            raise urllib.error.URLError("SECRET token and remote diagnostic")

        client, opener = self.client(commit_then_disconnect)
        self.assertCode("transport_error", client.board, "项目/Studiolo", "create", {"title": "test"})
        self.assertEqual(1, len(applied))
        self.assertEqual(1, sum(message["method"] == "tools/call" for message in opener.messages))
        self.assertEqual(2, json.loads(self.state.read_text())["counter"])

    def test_stale_session_clears_without_replay_next_call_reinitializes(self):
        replies = [Response(status=404), None]

        def handler(message):
            response = replies.pop(0)
            return response if response is not None else rpc(message, {"tools": []})

        client, opener = self.client(handler)
        self.assertCode("session_invalid", client.tools_list)
        self.assertNotIn("session_id", json.loads(self.state.read_text()))
        self.assertEqual(1, opener.initializations)
        self.assertEqual({"tools": []}, client.tools_list())
        self.assertEqual(2, opener.initializations)

    def test_redirect_and_proxy_are_disabled(self):
        client = McpClient("https://example.test/mcp", self.token, self.state)
        self.assertTrue(any(isinstance(handler, _NoRedirect) for handler in client._opener.handlers))
        self.assertFalse(any(isinstance(handler, urllib.request.ProxyHandler) and handler.proxies
                             for handler in client._opener.handlers))
        redirect = _NoRedirect()
        request = urllib.request.Request("https://example.test/mcp", b"{}")
        self.assertIsNone(redirect.redirect_request(request, None, 307, "redirect", {}, "https://other.test"))
        client._opener = FakeOpener(lambda message: Response(status=307))
        self.assertCode("redirect_refused", client.tools_list)

    def test_bounds_apply_to_request_and_response(self):
        client, opener = self.client(lambda message: Response(b" " * (MAX_BYTES + 1)))
        self.assertCode("response_too_large", client.tools_list)
        sends_before = len(opener.messages)
        self.assertCode("request_too_large", client.call, "tools/call", {"text": "x" * MAX_BYTES})
        self.assertEqual(sends_before, len(opener.messages))

    def test_invalid_board_content_is_safe(self):
        for content in [[], [{"type": "image", "text": "{}"}], [{"type": "text", "text": "SECRET invalid"}]]:
            client, _ = self.client(lambda message: rpc(message, {"content": content}))
            self.assertCode("invalid_board_response", client.board, "项目/Studiolo", "list", {})

    def test_all_calls_share_state_lock_across_instances_and_threads(self):
        opener = FakeOpener()

        def call(_):
            client = McpClient("http://127.0.0.1:43117/mcp", self.token, self.state)
            client._opener = opener
            return client.tools_list()

        with ThreadPoolExecutor(max_workers=8) as executor:
            self.assertEqual([{"tools": []}] * 24, list(executor.map(call, range(24))))
        self.assertEqual(1, opener.initializations)
        self.assertEqual(list(range(1, 26)), [item["id"] for item in opener.messages if "id" in item])

    def test_token_rotation_starts_new_session_without_copying_token(self):
        client, opener = self.client()
        client.tools_list()
        self.token.write_text("rotated-private-bearer")
        client.tools_list()
        self.assertEqual(2, opener.initializations)
        self.assertNotIn("rotated-private-bearer", self.state.read_text())

    def test_endpoints_fail_closed(self):
        for endpoint in ["http://example.test/mcp", "http://localhost/mcp", "http://[::1]/mcp",
                         "https://user:secret@example.test/mcp", "https://example.test/mcp#x",
                         "https://example.test/mcp?token=foo", "file:///mcp", "https://example.test:bad/mcp"]:
            self.assertCode("invalid_endpoint", McpClient, endpoint, self.token, self.state)

    def test_symlink_token_state_and_lock_rejected(self):
        client, opener = self.client()
        real = self.root / "real"
        real.write_text("unchanged")
        real.chmod(0o600)
        for path in [self.token, self.state, client.lock_file]:
            if path.exists():
                path.unlink()
            path.symlink_to(real)
            self.assertCode("unsafe_file", client.tools_list)
            self.assertEqual("unchanged", real.read_text())
            path.unlink()
            if path == self.token:
                path.write_text("private-example-bearer")
                path.chmod(0o600)
        self.assertEqual([], opener.messages)

    def test_public_files_parent_hardlink_and_corrupt_state_rejected(self):
        client, _ = self.client()
        self.token.chmod(0o644)
        self.assertCode("unsafe_file", client.tools_list)
        self.assertEqual(0o644, stat.S_IMODE(self.token.stat().st_mode))
        self.token.chmod(0o600)
        linked = self.root / "linked"
        os.link(self.token, linked)
        self.assertCode("unsafe_file", client.tools_list)
        linked.unlink()
        self.state.write_text("SECRET invalid state")
        self.state.chmod(0o600)
        self.assertCode("invalid_state", client.tools_list)
        public = self.root / "public"
        public.mkdir(mode=0o755)
        self.assertCode("unsafe_file", McpClient, client.endpoint, self.token, public / "state")


class StoreTests(PrivateFixture):
    def store(self):
        return Store(self.root / "notifications.sqlite")

    def test_enqueue_is_idempotent_and_conflicting_reuse_rejected(self):
        store = self.store()
        item = store.enqueue("request-1", "codex-local", "task-42")
        self.assertEqual(item, store.enqueue("request-1", "codex-local", "task-42"))
        for client, task in [("claude-local", "task-42"), ("codex-local", "task-43")]:
            with self.assertRaisesRegex(ValueError, "idempotency conflict"):
                store.enqueue("request-1", client, task)
        self.assertEqual([item], store.list())
        self.assertEqual(32, len(item["id"]))
        self.assertEqual("pending", item["state"])
        self.assertIsInstance(item["created_ms"], int)
        self.assertEqual(0o600, stat.S_IMODE(store.path.stat().st_mode))

    def test_pending_filter_and_atomic_claim_allow_only_one_sender(self):
        store = self.store()
        item = store.enqueue("request-1", "codex-local", "task-42")
        other = store.enqueue("request-2", "claude-local", "task-43")
        self.assertEqual([item], store.pending("codex-local"))
        with ThreadPoolExecutor(max_workers=8) as executor:
            results = list(executor.map(lambda _: store.transition(item["id"], "pending", "dispatching"), range(16)))
        self.assertEqual(1, sum(results))
        self.assertEqual([other], store.pending())
        self.assertEqual("dispatching", store.get(item["id"])["state"])

    def test_concurrent_enqueue_same_key_has_one_row(self):
        store = self.store()
        with ThreadPoolExecutor(max_workers=8) as executor:
            items = list(executor.map(lambda _: store.enqueue("shared", "codex-local", "task-42"), range(16)))
        self.assertEqual(1, len({item["id"] for item in items}))
        self.assertEqual(1, len(store.list()))

    def test_restart_recovery_preserves_unknown_outcome_and_does_not_resend(self):
        store = self.store()
        item = store.enqueue("request-1", "codex-local", "task-42")
        pending = store.enqueue("request-2", "codex-local", "task-43")
        store.transition(item["id"], "pending", "dispatching")
        restarted = self.store()
        self.assertEqual("dispatching", restarted.get(item["id"])["state"])
        self.assertEqual(1, restarted.recover_dispatching())
        self.assertEqual(0, restarted.recover_dispatching())
        self.assertEqual("uncertain", restarted.get(item["id"])["state"])
        self.assertEqual("restart_uncertain", restarted.get(item["id"])["detail"])
        self.assertEqual([pending], restarted.pending())
        with self.assertRaisesRegex(ValueError, "invalid notification transition"):
            restarted.transition(item["id"], "uncertain", "pending")

    def test_terminal_states_cannot_be_reopened_and_detail_is_not_a_log_sink(self):
        store = self.store()
        item = store.enqueue("request-1", "codex-local", "task-42")
        with self.assertRaisesRegex(ValueError, "invalid notification result code"):
            store.transition(item["id"], "pending", "dispatching", "SECRET task body")
        for before, after in [("pending", "dispatching"), ("dispatching", "sent"), ("sent", "completed")]:
            self.assertTrue(store.transition(item["id"], before, after, after))
        with self.assertRaisesRegex(ValueError, "invalid notification transition"):
            store.transition(item["id"], "completed", "pending")
        self.assertIsNone(store.get("does-not-exist"))
        self.assertFalse(store.transition("does-not-exist", "pending", "cancelled"))

    def test_store_refuses_symlinks_and_invalid_identifiers(self):
        target = self.root / "target"
        target.write_text("not a database")
        target.chmod(0o600)
        path = self.root / "notifications.sqlite"
        path.symlink_to(target)
        self.assertCode("unsafe_file", Store, path)
        self.assertEqual("not a database", target.read_text())
        path.unlink()
        store = self.store()
        for invalid in ["", "../secret", "task body with spaces", "a" * 161, None]:
            with self.assertRaisesRegex(ValueError, "invalid route identifier"):
                store.enqueue(invalid, "codex-local", "task-42")


if __name__ == "__main__":
    unittest.main(verbosity=2)
