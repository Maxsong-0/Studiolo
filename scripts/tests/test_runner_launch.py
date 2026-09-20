"""Launcher contract tests: AST-load definitions without executing main or real I/O."""
import ast
import contextlib
import io
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock


SOURCE = Path(__file__).resolve().parents[2] / 'deploy/runner/launch.py'


def functions():
    tree = ast.parse(SOURCE.read_text())
    definitions = ast.Module(
        body=[node for node in tree.body if isinstance(node, ast.FunctionDef)],
        type_ignores=[])
    namespace = {'json': json}
    exec(compile(definitions, str(SOURCE), 'exec'), namespace)
    return namespace


class ReceiptTests(unittest.TestCase):
    def setUp(self):
        self.extract = functions()['stopped_receipt']
        self.receipt = {'status': 'stopped', 'remaining': ['unfinished'], 'tool_calls': 2}

    def test_explicit_receipt_from_both_provider_event_shapes(self):
        text = json.dumps(self.receipt)
        for event in ({'result': text},
                      {'item': {'type': 'agent_message', 'text': text}},
                      {'message': {'content': [{'type': 'text', 'text': text}]}}):
            with self.subTest(event=event):
                self.assertEqual(self.extract(event), self.receipt)

    def test_arbitrary_json_shapes_never_throw_or_infer_receipt(self):
        for event in (None, [], 1, 'text', {'message': None},
                      {'message': {'content': None}}, {'message': {'content': 3}},
                      {'item': []}, {'result': {'status': 'stopped'}}, {'result': '\ud800'},
                      {'message': {'content': [{'type': 'text', 'text': 4}]}}):
            with self.subTest(event=event):
                self.assertIsNone(self.extract(event))

    def test_prose_fences_arrays_and_non_stopped_status_do_not_count(self):
        for text in ('done', '```json\n{"status":"stopped"}\n```',
                     '[{"status":"stopped"}]', '{"status":"completed"}',
                     '{"status":"stopped"} trailing'):
            with self.subTest(text=text):
                self.assertIsNone(self.extract({'result': text}))

    def test_receipt_limit_counts_utf8_bytes_and_does_not_truncate(self):
        boundary = '{"status":"stopped","padding":"' + 'x' * 131039 + '"}'
        self.assertEqual(len(boundary.encode()), 131072)
        self.assertIsNotNone(self.extract({'result': boundary}))
        self.assertIsNone(self.extract({'result': boundary + ' '}))
        multibyte = json.dumps({'status': 'stopped', 'padding': '中' * 50000},
                               ensure_ascii=False)
        self.assertLess(len(multibyte), 131072)
        self.assertIsNone(self.extract({'result': multibyte}))


class LauncherHarness:
    """Already-exited fake child with unread pipe data; every OS surface is replaced."""
    def __init__(self, chunks, *, cancelled=False, result=0, provider='codex'):
        self.namespace = functions()
        self.chunks = list(chunks) + [b'']
        self.clock = 0.0
        self.child = SimpleNamespace(
            pid=42, stdin=io.BytesIO(), stdout=SimpleNamespace(fileno=lambda: 9),
            poll=Mock(return_value=result), wait=Mock(return_value=result),
            send_signal=Mock())
        self.registered = {}
        selector = SimpleNamespace(
            register=lambda file, _: self.registered.update({9: file}),
            unregister=lambda _: self.registered.pop(9),
            get_map=lambda: self.registered,
            select=lambda **_: [(SimpleNamespace(fileobj=self.child.stdout), 1)])
        job = {'provider': provider, 'args': [], 'instruction': 'fixture',
               'budget': {'pacing_tools': 24, 'wrapup_tools': 120, 'max_tools': 160,
                          'wrapup_seconds': 1500, 'max_seconds': 2100}}
        self.server = Mock()
        self.killpg = Mock()
        self.path = lambda name: SimpleNamespace(
            read_bytes=lambda: json.dumps(job).encode(),
            read_text=lambda: 'a' * 64,
            exists=lambda: cancelled and name == '/inputs/cancel-requested')
        self.namespace.update({
            'MAX_OUTPUT': 16 * 1024 * 1024, 'CAPABILITY': '', 'PROVIDER_PATH': '',
            'pathlib': SimpleNamespace(Path=self.path),
            'http': SimpleNamespace(server=SimpleNamespace(HTTPServer=lambda *_: self.server)),
            'Relay': object, 'threading': SimpleNamespace(Thread=lambda **_: Mock()),
            'subprocess': SimpleNamespace(Popen=lambda *_, **__: self.child,
                                          PIPE=-1, DEVNULL=-3),
            'signal': SimpleNamespace(signal=Mock(), SIGTERM=15, SIGINT=2, SIGKILL=9),
            'time': SimpleNamespace(monotonic=self.monotonic),
            'selectors': SimpleNamespace(DefaultSelector=lambda: selector, EVENT_READ=1),
            'os': SimpleNamespace(read=lambda *_: self.chunks.pop(0), killpg=self.killpg),
        })

    def monotonic(self):
        self.clock += 0.01
        return self.clock

    def run(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            status = self.namespace['main']()
        return status, [json.loads(line) for line in output.getvalue().splitlines()]


class DrainTests(unittest.TestCase):
    @staticmethod
    def tool(identity='call-1'):
        return json.dumps({'item': {'id': identity, 'type': 'command_execution'}}).encode()

    def test_exited_child_is_drained_and_duplicate_tool_ids_count_once(self):
        line = self.tool() + b'\n'
        harness = LauncherHarness([line[:9], line[9:] + line], result=7)
        status, output = harness.run()
        self.assertEqual(status, 7)
        self.assertEqual(output[-1]['tool_calls'], 1)
        self.assertEqual(output[-1]['edits'], 0)
        harness.killpg.assert_not_called()

    def test_complete_final_json_without_newline_is_counted(self):
        status, output = LauncherHarness([self.tool()], result=7).run()
        self.assertEqual(status, 7)
        self.assertEqual(output[-1]['tool_calls'], 1)

    def test_cancelled_exit_preserves_receipt_from_final_unterminated_line(self):
        receipt = {'status': 'stopped', 'remaining': ['unfinished']}
        event = json.dumps({'item': {'type': 'agent_message',
                                     'text': json.dumps(receipt)}}).encode()
        harness = LauncherHarness([event], cancelled=True, result=-2)
        status, output = harness.run()
        self.assertEqual(status, 130)
        self.assertEqual([value['stopped_receipt'] for value in output
                          if 'stopped_receipt' in value], [receipt])
        harness.child.send_signal.assert_called_once_with(2)

    def test_malformed_json_and_event_shapes_are_typed_protocol_exits(self):
        for data in (b'{partial', b'[]\n', b'null\n',
                     b'{"message":null}\n', b'{"message":{"content":null}}\n',
                     b'{"message":{"content":{}}}\n'):
            with self.subTest(data=data):
                status, _ = LauncherHarness([data]).run()
                self.assertEqual(status, 72)

    def test_output_overflow_is_distinct_and_does_not_publish_partial_event(self):
        harness = LauncherHarness([self.tool() + b'\n'])
        harness.namespace['MAX_OUTPUT'] = 8
        status, output = harness.run()
        self.assertEqual(status, 71)
        self.assertEqual(output, [])
        harness.killpg.assert_called_once_with(42, 9)


if __name__ == '__main__':
    unittest.main()
