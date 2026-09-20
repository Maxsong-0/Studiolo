#!/usr/bin/python3
"""Pinned supervisor and loopback-to-Unix relay. Contains no long-lived credential.
The outer namespace/cgroup and Gateway remain the security boundaries.
"""
import hashlib
import http.client
import http.server
import json
import os
import pathlib
import selectors
import signal
import socket
import subprocess
import threading
import time

MAX_BODY = 4 * 1024 * 1024
MAX_OUTPUT = 16 * 1024 * 1024
CAPABILITY = ''
PROVIDER_PATH = ''


class UnixHTTP(http.client.HTTPConnection):
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(310)
        self.sock.connect('/run/studiolo-gateway.sock')


class Relay(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        try:
            length = int(self.headers.get('Content-Length', '-1'))
            if self.path != PROVIDER_PATH or not 0 < length <= MAX_BODY:
                self.send_error(403)
                return
            body = self.rfile.read(length)
            if len(body) != length:
                self.send_error(400)
                return
            connection = UnixHTTP('localhost', timeout=310)
            # Never forward CLI-supplied authentication, target or tracing headers.
            connection.request('POST', self.path, body, {
                'Authorization': 'Bearer ' + CAPABILITY,
                'Content-Type': 'application/json',
                'X-Studiolo-Call-Id': hashlib.sha256(self.path.encode() + b'\0' + body).hexdigest(),
            })
            response = connection.getresponse()
            data = response.read(MAX_OUTPUT + 1)
            if len(data) > MAX_OUTPUT:
                self.send_error(502)
                return
            self.send_response(response.status)
            self.send_header('Content-Type', response.getheader('Content-Type', 'application/json'))
            self.send_header('Content-Length', str(len(data)))
            self.end_headers()
            self.wfile.write(data)
            connection.close()
        except (OSError, ValueError, http.client.HTTPException):
            self.send_error(502)


def user_message(text):
    return json.dumps({'type': 'user', 'message': {'role': 'user', 'content': text}}) + '\n'


def stopped_receipt(event):
    """Accept only an explicit JSON object supplied by the CLI, never infer success."""
    if not isinstance(event, dict):
        return None
    texts = []
    if isinstance(event.get('result'), str):
        texts.append(event['result'])
    item = event.get('item', {})
    if isinstance(item, dict) and item.get('type') == 'agent_message':
        texts.append(item.get('text', ''))
    message = event.get('message', {})
    if isinstance(message, dict) and isinstance(message.get('content', []), list):
        for block in message.get('content', []):
            if isinstance(block, dict) and block.get('type') == 'text':
                texts.append(block.get('text', ''))
    for text in texts:
        if not isinstance(text, str):
            continue
        try:
            if len(text.encode()) > 131072:
                continue
        except UnicodeError:
            continue
        try:
            value = json.loads(text)
        except ValueError:
            continue
        if isinstance(value, dict) and value.get('status') == 'stopped':
            return value
    return None


def main():
    global CAPABILITY, PROVIDER_PATH
    job = json.loads(pathlib.Path('/inputs/job.json').read_bytes())
    CAPABILITY = pathlib.Path('/run/job-capability').read_text().strip()
    if len(CAPABILITY) != 64 or any(c not in '0123456789abcdef' for c in CAPABILITY):
        return 70
    claude = job['provider'] == 'claude'
    PROVIDER_PATH = '/v1/messages' if claude else '/v1/responses'
    server = http.server.HTTPServer(('127.0.0.1', 8765), Relay)
    server.timeout = 1
    threading.Thread(target=server.serve_forever, daemon=True).start()
    environment = {'PATH': '/usr/local/bin:/usr/bin:/bin', 'HOME': '/home/runner', 'LANG': 'C.UTF-8',
                   'CODEX_HOME': '/home/runner/.codex', 'CLAUDE_CONFIG_DIR': '/home/runner/.claude',
                   'STUDIOLO_JOB_CAPABILITY': CAPABILITY, 'ANTHROPIC_API_KEY': CAPABILITY,
                   'ANTHROPIC_BASE_URL': 'http://127.0.0.1:8765',
                   'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC': '1'}
    child = subprocess.Popen([job['provider']] + job['args'], stdin=subprocess.PIPE,
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=environment,
                             start_new_session=True, cwd='/work')
    prompt = job['instruction'] + '\nReturn candidate changes. Do not launch child agents. On cancellation, emit only a JSON stopped BoardReceipt: status, artifacts, conflicts, unlanded, acceptance (criterion/result/evidence), remaining, elapsed_ms, tool_calls, learned, closeout (learned/adr_required/rule_change_required/destinations). Use actual evidence, mark unfinished criteria unverified, and never claim owner acceptance. A missing or invalid receipt becomes abandoned after the grace period.'
    child.stdin.write((user_message(prompt) if claude else prompt).encode())
    child.stdin.flush()
    if not claude:
        child.stdin.close()
    cancelled = [None]

    def cancel(*_):
        if cancelled[0] is None:
            cancelled[0] = time.monotonic()
            if claude:
                try:
                    child.stdin.write(user_message('Cancelled: submit only a stopped receipt; no further tools.').encode())
                    child.stdin.flush()
                except OSError:
                    pass
            else:
                child.send_signal(signal.SIGINT)

    signal.signal(signal.SIGTERM, cancel)
    selector = selectors.DefaultSelector()
    selector.register(child.stdout, selectors.EVENT_READ)
    start, size, pending, tools, edits, phase = time.monotonic(), 0, b'', 0, 0, 'working'
    seen = set()
    exited_at = None
    while child.poll() is None or selector.get_map():
        if pathlib.Path('/inputs/cancel-requested').exists():
            cancel()
        if child.poll() is not None:
            exited_at = exited_at or time.monotonic()
            if time.monotonic() - exited_at > 5:
                try:
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                return 72
        elapsed = time.monotonic() - start
        budget = job['budget']
        next_phase = ('exhausted' if tools >= budget['max_tools'] or elapsed >= budget['max_seconds'] else
                      'wrapup' if tools >= budget['wrapup_tools'] or elapsed >= budget['wrapup_seconds'] else
                      'pacing' if tools >= budget['pacing_tools'] else 'working')
        if next_phase != phase:
            phase = next_phase
            if phase == 'exhausted':
                cancel()
            elif claude:
                try:
                    child.stdin.write(user_message('Budget phase: ' + phase + '. Prepare a bounded receipt.').encode())
                    child.stdin.flush()
                except OSError:
                    pass
        if cancelled[0] is not None and time.monotonic() - cancelled[0] >= 180:
            os.killpg(child.pid, signal.SIGKILL)
            break
        for key, _ in selector.select(timeout=0.2):
            chunk = os.read(key.fileobj.fileno(), 65536)
            if not chunk:
                selector.unregister(key.fileobj)
                if not pending:
                    continue
                # JSONL permits a complete final record without a trailing newline.
                chunk = b"\n"
            size += len(chunk)
            if size > MAX_OUTPUT:
                os.killpg(child.pid, signal.SIGKILL)
                return 71
            pending += chunk
            while b'\n' in pending:
                line, pending = pending.split(b'\n', 1)
                try:
                    event = json.loads(line)
                except ValueError:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.wait(timeout=5)
                    return 72
                if not isinstance(event, dict):
                    os.killpg(child.pid, signal.SIGKILL)
                    return 72
                receipt = stopped_receipt(event)
                if receipt is not None:
                    print(json.dumps({'stopped_receipt': receipt}), flush=True)
                item = event.get('item', {})
                message = event.get('message', {})
                if not isinstance(message, dict) or not isinstance(message.get('content', []), list):
                    os.killpg(child.pid, signal.SIGKILL)
                    return 72
                candidates = message.get('content', [])
                if item:
                    candidates = candidates + [item]
                for item in candidates:
                    if not isinstance(item, dict):
                        continue
                    identity = item.get('id')
                    kind = item.get('type')
                    if isinstance(identity, str) and identity and identity not in seen and kind in ('tool_use', 'command_execution', 'file_change', 'mcp_tool_call'):
                        seen.add(identity)
                        tools += 1
                        edits += int(kind == 'file_change' or item.get('name') in ('Edit', 'Write'))
                print(json.dumps({'tool_calls': tools, 'edits': edits, 'phase': phase}), flush=True)
    result = child.wait(timeout=5)
    server.shutdown()
    return result if result >= 0 else 128 - result


if __name__ == '__main__':
    raise SystemExit(main())
