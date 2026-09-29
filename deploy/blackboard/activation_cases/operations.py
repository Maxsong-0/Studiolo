"""真实操作封装的注入式契约。"""
from .fixture import *


class OperationTests(unittest.TestCase):
    def operation(self):
        result = Operations('0' * 64, assembly, c, Mock(), legacy)
        result.old_units = {'studiolo-human': b'old human', 'studiolo-agent': b'old agent'}
        result.new_units = {'studiolo-human': b'new human', 'studiolo-agent': b'new agent'}
        result.old_shas = {n: c.sha(v) for n, v in result.old_units.items()}
        result.new_shas = {n: c.sha(v) for n, v in result.new_units.items()}
        result.human_sha = '2' * 64
        result.old_human_sha = '3' * 64
        return result

    def test_agent_probe_is_fixed_anonymous_head_without_proxy_redirect_or_body(self):
        ops = self.operation()
        ops.runtime.command.return_value.stdout = '401'
        with patch.object(ops_module.time, 'monotonic', return_value=10):
            self.assertTrue(ops.agent_http_ready(20))
        self.assertEqual(ops.runtime.command.call_args.args, ([
            '/usr/bin/curl', '--disable', '--head', '--silent',
            '--output', '/dev/null', '--write-out', '%{http_code}',
            '--noproxy', '*', '--proxy', '', '--proto', '=http',
            '--max-redirs', '0', '--max-time', '1.0',
            '--url', 'http://127.0.0.1:4400/mcp'], 1.0))

    def test_agent_probe_refuses_non401_and_unready_connections(self):
        ops = self.operation()
        with patch.object(ops_module.time, 'monotonic', return_value=10):
            for status in ('200', '301', '302', '403', '500', '401\n', '000'):
                with self.subTest(status=status):
                    ops.runtime.command.return_value.stdout = status
                    self.assertFalse(ops.agent_http_ready(20))
            for error in (ConnectionRefusedError(), ops_module.subprocess.TimeoutExpired('curl', 1),
                          ops_module.subprocess.CalledProcessError(7, 'curl')):
                with self.subTest(error=type(error).__name__):
                    ops.runtime.command.side_effect = error
                    self.assertFalse(ops.agent_http_ready(20))

    def test_agent_probe_never_runs_after_deadline_and_bounds_last_attempt(self):
        ops = self.operation()
        with patch.object(ops_module.time, 'monotonic', return_value=20):
            self.assertFalse(ops.agent_http_ready(20))
        ops.runtime.command.assert_not_called()
        ops.runtime.command.return_value.stdout = '401'
        with patch.object(ops_module.time, 'monotonic', side_effect=[19.75, 20.01]):
            self.assertFalse(ops.agent_http_ready(20))
        self.assertEqual(ops.runtime.command.call_args.args[1], 0.25)
        args = ops.runtime.command.call_args.args[0]
        self.assertEqual(args[args.index('--max-time') + 1], '0.25')

    def agent_poll(self, ops, statuses, *, never_ready=False):
        ticks = [0.0]
        ops.agent_binary = Path('/fictional/bin/studiolo-agent')
        ops.agent_sha = c.sha(b'agent')
        ops.runtime.wait_program.side_effect = lambda binary, **kwargs: runtime.wait_program(
            binary, readlink=lambda _: str(binary), **kwargs)
        attempts = iter(statuses)
        def probe(*_args):
            status = '503' if never_ready else next(attempts)
            if isinstance(status, BaseException):
                raise status
            return types.SimpleNamespace(stdout=status)
        ops.runtime.command.side_effect = probe
        state = dict(MainPID='123', ActiveState='active', SubState='running', NRestarts='0')
        with patch.object(ops_module.time, 'monotonic', side_effect=lambda: ticks[0]), \
                patch.object(ops_module.time, 'sleep', side_effect=lambda n: ticks.__setitem__(0, ticks[0] + n)), \
                patch.object(ops, 'properties', return_value=state), patch.object(ops, 'read_agent', return_value=b'agent'), \
                patch.object(os, 'stat', return_value=types.SimpleNamespace(st_dev=1, st_ino=2)):
            result = ops.agent_ready()
        return result, ticks[0]

    def test_agent_process_waits_for_actual_401_after_connection_and_status_failures(self):
        ops = self.operation()
        result, elapsed = self.agent_poll(ops, [ConnectionRefusedError(), '503', '401'])
        self.assertEqual(result, 123)
        self.assertEqual(ops.runtime.command.call_count, 3)
        self.assertGreaterEqual(elapsed, 0.2)

    def test_agent_process_without_http_ready_hits_deadline(self):
        ops = self.operation()
        with self.assertRaises(ValueError):
            self.agent_poll(ops, [], never_ready=True)
        self.assertGreater(ops.runtime.command.call_count, 1)
        self.assertLessEqual(ops.runtime.command.call_count, 201)

    def test_actual_stop_order_and_no_socket_removal(self):
        ops = self.operation()
        with patch.object(ops, 'known_units'), patch.object(ops, 'stopped'):
            ops.stop()
        self.assertEqual([call.args[0] for call in ops.runtime.command.call_args_list], [
            ['systemctl', 'stop', 'studiolo-agent.service'], ['systemctl', 'stop', 'studiolo-human.service']])

    def test_worker_uses_original_code_as_unprivileged_isolated_stdin(self):
        ops = self.operation()
        summary = {'epoch': 1, 'factor_ref': 'studiolo-human-totp-v2', 'code_count': 10, 'restore_event_count': 0}
        with patch.object(ops_module.pwd, 'getpwnam', return_value=types.SimpleNamespace(pw_uid=1000, pw_gid=1000)), \
                patch.object(ops, 'trusted_read', return_value=b'# fictional worker') as read, \
                patch.object(ops_module.subprocess, 'run', return_value=types.SimpleNamespace(
                    stdout=json.dumps(summary).encode())) as run:
            self.assertEqual(ops.worker('binding'), summary)
        read.assert_called_once_with(legacy.WORKER)
        self.assertEqual(run.call_args.args[0], ['/usr/bin/python3', '-I', '-B', '-S', '-', 'binding'])
        self.assertEqual(run.call_args.kwargs['user'], 1000)
        self.assertEqual(run.call_args.kwargs['extra_groups'], [])
        self.assertEqual(run.call_args.kwargs['timeout'], 40)
        self.assertEqual(run.call_args.kwargs['pass_fds'], ())

    def test_human_must_be_ready_with_board_socket_before_agent_can_start(self):
        ops = self.operation()
        events = []
        ops.runtime.command.side_effect = lambda args, *rest: events.append(args[-1])
        ops.runtime.ready.side_effect = lambda *args: events.append('human-ready')
        with patch.object(ops, 'board_ready', side_effect=lambda: events.append('board-ready')), \
                patch.object(ops, 'agent_ready', side_effect=lambda: events.append('agent-ready')):
            ops.start_human()
            ops.start_agent()
        self.assertEqual(events, ['studiolo-human.service', 'human-ready', 'board-ready',
                                  'studiolo-agent.service', 'agent-ready'])

    def test_actual_recover_partial_install_restores_only_changed_unit(self):
        ops = self.operation()
        current = {'studiolo-human': ops.new_shas['studiolo-human'], 'studiolo-agent': ops.old_shas['studiolo-agent']}
        events = []
        ops.runtime.command.side_effect = lambda args, *rest: events.append(args)
        with patch.object(ops, 'known_units', return_value=current), \
                patch.object(ops, 'stop', side_effect=lambda: events.append('stop')) , \
                patch.object(ops, 'unchanged', side_effect=lambda: events.append('binding')), \
                patch.object(c, 'atomic_unit', side_effect=lambda *args: events.append('restore-human')), \
                patch.object(ops, 'agent_ready'), patch.object(ops, 'units', return_value=ops.old_units):
            self.assertEqual(ops.recover(), 'old_programs_restored')
        self.assertEqual(events, ['stop', 'binding', 'restore-human', ['systemctl', 'daemon-reload'],
            ['systemctl', 'start', 'studiolo-human.service'], ['systemctl', 'start', 'studiolo-agent.service'], 'binding'])

    def test_unknown_unit_refuses_recovery_without_service_actions(self):
        ops = self.operation()
        with patch.object(ops, 'units', return_value={**ops.new_units, 'studiolo-agent': b'unknown'}), \
                patch.object(c, 'atomic_unit') as replace:
            with self.assertRaises(ValueError):
                ops.recover()
        replace.assert_not_called()
        ops.runtime.command.assert_not_called()

    def test_changed_auth_binding_stops_but_does_not_restore_or_restart(self):
        ops = self.operation()
        with patch.object(ops, 'known_units', return_value=ops.new_shas), patch.object(ops, 'stop') as stop, \
                patch.object(ops, 'unchanged', side_effect=ValueError('changed')), \
                patch.object(c, 'atomic_unit') as replace:
            with self.assertRaises(ValueError):
                ops.recover()
        stop.assert_called_once()
        replace.assert_not_called()
        ops.runtime.command.assert_not_called()

    def test_preflight_rejects_wrong_live_unit_before_service_command(self):
        ops = self.operation()
        with patch.object(ops_module.sys, 'platform', 'linux'), patch.object(os, 'getuid', return_value=0), \
                patch.object(os, 'geteuid', return_value=0), patch.object(ops_module.pwd, 'getpwnam',
                return_value=types.SimpleNamespace(pw_uid=1000)), \
                patch.object(ops_module, 'verify_candidate', return_value={}), \
                patch.object(ops, 'units', return_value={'studiolo-human': b'wrong', 'studiolo-agent': b'wrong'}):
            with self.assertRaises(ValueError):
                ops.preflight()
        ops.runtime.command.assert_not_called()

    def test_stopped_refuses_lingering_socket_without_removing_it(self):
        ops = self.operation()
        with patch.object(ops, 'properties', return_value={'ActiveState': 'inactive', 'MainPID': '0'}), \
                patch.object(os.path, 'lexists', return_value=True), patch.object(os, 'unlink') as unlink:
            with self.assertRaises(ValueError):
                ops.stopped()
        unlink.assert_not_called()


