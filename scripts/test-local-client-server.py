#!/usr/bin/env python3
"""Companion boundaries and failure/recovery contracts; no native model invocation."""
import copy
import json
import os
from pathlib import Path
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import urllib.error
import urllib.request

from local_client.config import load, new_token, private_create
from local_client.mcp import McpError
from local_client.server import Companion, Conflict, Handler, ThreadingHTTPServer

TASK = '01M2YH2E6QAV58N0Q4XK22VBDP'
OTHER = '01M2YH2E6QAV58N0Q4XK22VBDQ'


class Upstream:
    def __init__(self, *_):
        self.status = 'draft'
        self.revoked = False
        self.calls = []
        self.fail_create = False
        self.old = False

    def board(self, project, operation, request):
        self.calls.append((project, operation, request))
        if self.revoked or (operation == 'create' and self.fail_create):
            raise McpError('test_transport_failure')
        if operation == 'create':
            return TASK
        return {'task': {'task_id': TASK, 'status': self.status, 'spec': {'project': project, 'adapter': 'consumer'}},
                'uses_old_policy': self.old, 'board': {'receipt': {'status': 'completed'} if self.status == 'succeeded' else None}}

    def call(self, method, params):
        if self.revoked:
            raise McpError('revoked')
        if method == 'tools/list':
            return {'tools': [{'name': n} for n in ['board', 'handshake', 'propose_desired']]}
        return {'content': [{'type': 'text', 'text': '{}'}]}


class ServerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.root.chmod(0o700)
        self.cfg = {'port': 4318, 'origin': 'http://127.0.0.1:5173', 'pairing_token': 'p' * 43,
                    'state_directory': self.root, 'clients': [{'id': 'c', 'provider': 'codex', 'mode': 'codex-queue',
                    'project': '项目/Studiolo', 'cwd': str(self.root), 'mcp_endpoint': 'http://127.0.0.1:4319/mcp',
                    'mcp_token_file': str(self.root / 'upstream'), 'bridge_token': 'b' * 43}]}
        self.up = patch('local_client.server.McpClient', Upstream)
        self.auth = patch('local_client.server.subscription_status', return_value={'ready': True, 'reason': 'test'})
        self.up.start(); self.auth.start()
        self.app = Companion(self.cfg)
        self.app.seen['c'] = time.monotonic()

    def tearDown(self):
        self.app.close()
        self.up.stop(); self.auth.stop()
        self.temp.cleanup()

    def dispatch(self, tid=TASK, rid='request'):
        return self.app.dispatch({'client_id': 'c', 'task_id': tid, 'request_id': rid})

    def creation(self, rid='create'):
        return {'client_id': 'c', 'request_id': rid, 'title': 'read only', 'goal': 'read and report', 'acceptance_criteria': ['read done']}

    def test_second_daemon_cannot_recover_live_dispatch(self):
        item = self.dispatch()
        self.app.store.transition(item['id'], 'pending', 'dispatching')
        with self.assertRaises(ValueError):
            Companion(self.cfg)
        self.assertEqual(self.app.store.get(item['id'])['state'], 'dispatching')

    def test_duplicate_ids_and_repeat_clicks_never_requeue(self):
        item = self.dispatch()
        self.app.store.transition(item['id'], 'pending', 'dispatching')
        self.app.store.transition(item['id'], 'dispatching', 'sent')
        self.app.mcp['c'].status = 'running'
        self.assertEqual(self.dispatch()['id'], item['id'])
        self.assertEqual(self.dispatch(rid='new-click')['id'], item['id'])
        with self.assertRaises(Conflict):
            self.dispatch(OTHER)
        self.assertEqual(len(self.app.store.list()), 1)

    def test_task_authority_and_disconnected_client_fail_closed(self):
        self.app.mcp['c'].status = 'running'
        with self.assertRaises(Conflict): self.dispatch()
        self.app.mcp['c'].status = 'draft'
        self.app.mcp['c'].old = True
        with self.assertRaises(Conflict): self.dispatch()
        self.app.mcp['c'].old = False
        self.app.mcp['c'].revoked = True
        with self.assertRaises(McpError): self.dispatch()
        self.app.mcp['c'].revoked = False
        self.app.seen.clear()
        with self.assertRaises(Conflict): self.dispatch()
        self.assertEqual(self.app.store.list(), [])

    def test_created_tasks_are_readonly_and_idempotent(self):
        body = self.creation()
        first = self.app.create(body)
        self.assertEqual(self.app.create(body)['task_id'], first['task_id'])
        creates = [c for c in self.app.mcp['c'].calls if c[1] == 'create']
        self.assertEqual(len(creates), 1)
        self.assertEqual(creates[0][2]['spec']['files_owned'], [])
        with self.assertRaises(Conflict):
            self.app.create({**body, 'goal': 'different'})

    def test_unknown_create_reply_is_never_replayed(self):
        self.app.mcp['c'].fail_create = True
        with self.assertRaises(McpError): self.app.create(self.creation())
        self.app.mcp['c'].fail_create = False
        with self.assertRaises(Conflict): self.app.create(self.creation())
        self.assertEqual(sum(c[1] == 'create' for c in self.app.mcp['c'].calls), 1)

    def test_proxy_cannot_write_undispatched_or_cross_project_task(self):
        def call(project, tid, op='claim'):
            return self.app.proxy('c', {'method': 'tools/call', 'params': {'name': 'board', 'arguments': {
                'project': project, 'operation': op, 'request': {'task_id': tid}}}})
        with self.assertRaises(Conflict): call('项目/Studiolo', TASK)
        self.dispatch()
        self.assertIn('content', call('项目/Studiolo', TASK))
        with self.assertRaises(ValueError): call('项目/Other', TASK)
        with self.assertRaises(Conflict): call('项目/Studiolo', OTHER)
        with self.assertRaises(ValueError): call('项目/Studiolo', TASK, 'create')
        tools = self.app.proxy('c', {'method': 'tools/list', 'params': {}})['tools']
        self.assertNotIn('propose_desired', [t['name'] for t in tools])

    def test_receipt_not_notification_defines_completion(self):
        item = self.dispatch()
        self.app.store.transition(item['id'], 'pending', 'dispatching')
        self.app.store.transition(item['id'], 'dispatching', 'sent')
        self.app.reconcile()
        self.assertEqual(self.app.store.get(item['id'])['state'], 'sent')
        self.app.mcp['c'].status = 'succeeded'
        self.app.reconcile()
        self.assertEqual(self.app.store.get(item['id'])['state'], 'completed')

    def test_cancelled_task_removes_pending_notification(self):
        item = self.dispatch()
        self.app.mcp['c'].status = 'cancelled'
        self.app.reconcile()
        self.assertEqual(self.app.store.get(item['id'])['state'], 'cancelled')

    def test_claude_channel_is_one_shot_and_requires_ack(self):
        self.app.clients['c'].update(provider='claude', mode='claude-channel')
        item = self.dispatch()
        with patch('local_client.server.notification', return_value='task ID doorbell'):
            self.assertEqual(len(self.app.channel_poll('c')['events']), 1)
            self.assertEqual(self.app.channel_poll('c')['events'], [])
        self.assertEqual(self.app.store.get(item['id'])['state'], 'dispatching')

    def test_http_origin_auth_and_client_token_scope(self):
        server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        server.companion = self.app
        worker = threading.Thread(target=server.serve_forever); worker.start()
        try:
            base = f'http://127.0.0.1:{server.server_port}'
            def request(path, token, origin=None, body=None, host=None):
                headers = {'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'}
                if origin: headers['Origin'] = origin
                if host: headers['Host'] = host
                req = urllib.request.Request(base + path, headers=headers, data=None if body is None else json.dumps(body).encode())
                try:
                    with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(req) as res:
                        return res.status, json.load(res)
                except urllib.error.HTTPError as e:
                    with e:
                        return e.code, json.load(e)
            self.assertEqual(request('/v1/clients', 'p'*43)[0], 200)
            self.assertEqual(request('/v1/clients', 'bad')[0], 401)
            self.assertEqual(request('/v1/clients', 'p'*43, 'https://evil.invalid')[0], 403)
            self.assertEqual(request('/v1/clients', 'p'*43, host='evil.invalid')[0], 403)
            self.assertEqual(request('/v1/clients', 'b'*43)[0], 401)
            self.assertEqual(request('/v1/client/c/poll', 'p'*43, body={})[0], 401)
            self.assertEqual(request('/v1/client/c/poll', 'b'*43, body={})[0], 200)
            body = {'client_id': 'c', 'task_id': TASK, 'request_id': 'http'}
            self.assertEqual(request('/v1/dispatch', 'p'*43, self.cfg['origin'], body)[0], 200)
        finally:
            server.shutdown(); worker.join(); server.server_close()


class RegistrationTests(unittest.TestCase):
    def test_cli_registers_two_clients_without_overwriting_existing_identity(self):
        import subprocess
        import sys
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            root.chmod(0o700)
            secret = root / 'studiolo.token'
            private_create(secret, 'scoped-studiolo-test-token\n')
            config = root / 'config.json'
            cli = [sys.executable, str(Path(__file__).with_name('studiolo-local.py')), '--config', str(config)]
            common = ['--binary', '/bin/echo', '--cwd', str(root), '--endpoint', 'http://127.0.0.1:4321/mcp',
                      '--studiolo-token-file', str(secret), '--paid-usage-disabled']
            first = subprocess.run(cli + ['init', '--client', 'codex', '--provider', 'codex', '--thread',
                '00000000-0000-4000-8000-000000000000', '--origin', 'http://127.0.0.1:5173'] + common, capture_output=True)
            self.assertEqual(first.returncode, 0, first.stderr)
            original = load(config)['clients'][0]
            second = subprocess.run(cli + ['add-client', '--client', 'claude', '--provider', 'claude'] + common, capture_output=True)
            self.assertEqual(second.returncode, 0, second.stderr)
            current = load(config)
            self.assertEqual([c['id'] for c in current['clients']], ['codex', 'claude'])
            self.assertEqual(current['clients'][0], original)
            duplicate = subprocess.run(cli + ['add-client', '--client', 'claude', '--provider', 'claude'] + common, capture_output=True)
            self.assertNotEqual(duplicate.returncode, 0)
            self.assertEqual(len(load(config)['clients']), 2)


if __name__ == '__main__':
    unittest.main(verbosity=2)
