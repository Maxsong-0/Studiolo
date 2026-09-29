"""切换顺序与失败恢复契约。"""
from .fixture import *


class FakeOps:
    build = assembly.BUILD

    def __init__(self, fail=None):
        self.fail, self.record, self.events = fail, None, []

    def event(self, name):
        self.events.append(name)
        if name == self.fail:
            raise ValueError('secret exception must not be emitted')

    def begin(self):
        self.event('record')
        self.record = Path('/fictional/record')

    def preflight(self):
        self.event('preflight')

    def stop(self):
        self.event('stop-agent')
        self.event('stop-human')

    def backup(self):
        self.event('backup')

    def install(self):
        self.event('install')

    def start_human(self):
        self.event('start-human')

    def start_agent(self):
        self.event('start-agent')

    def accept(self):
        self.event('accept')
        return {'board_authorization_accepted': False}

    def receipt(self, name, value):
        self.event('receipt-' + name)

    def recover(self):
        self.event('recover')
        return 'old_programs_restored'

    def close(self):
        self.event('close')


class DeploymentTests(unittest.TestCase):
    def test_success_order_and_limited_acceptance(self):
        ops = FakeOps()
        code, result = activation.deploy(ops)
        self.assertEqual(code, 0)
        self.assertEqual(ops.events, ['record', 'preflight', 'stop-agent', 'stop-human', 'backup',
            'install', 'start-human', 'start-agent', 'accept', 'receipt-accepted', 'close'])
        self.assertFalse(result['database_rollback'])
        self.assertFalse(result['board_authorization_accepted'])

    def test_preflight_failure_never_stops_services(self):
        ops = FakeOps('preflight')
        with patch.object(signal, 'signal'):
            code, result = activation.deploy(ops)
        self.assertEqual(code, 1)
        self.assertEqual(result['program_recovery'], 'not_needed')
        self.assertNotIn('stop-agent', ops.events)
        self.assertNotIn('recover', ops.events)
        self.assertNotIn('secret', json.dumps(result))

    def test_backup_human_agent_install_and_stop_failure_all_recover(self):
        for fail in ('stop-agent', 'stop-human', 'backup', 'install', 'start-human', 'start-agent', 'accept'):
            with self.subTest(fail=fail), patch.object(signal, 'signal') as signals:
                ops = FakeOps(fail)
                code, result = activation.deploy(ops)
                self.assertEqual(code, 1)
                self.assertEqual(result['program_recovery'], 'old_programs_restored')
                self.assertEqual(ops.events[-3:], ['recover', 'receipt-failed', 'close'])
                self.assertEqual(signals.call_count, 3)

    def test_failed_recovery_reports_operator_boundary(self):
        ops = FakeOps('start-agent')
        ops.recover = Mock(side_effect=ValueError('do not expose'))
        with patch.object(signal, 'signal'):
            code, result = activation.deploy(ops)
        self.assertEqual(result['program_recovery'], 'blocked_requires_operator')
        self.assertFalse(result['database_rollback'])

    def test_interrupt_after_stop_enters_bounded_recovery(self):
        ops = FakeOps()
        ops.backup = Mock(side_effect=InterruptedError('interrupted'))
        with patch.object(signal, 'signal') as signals:
            code, result = activation.deploy(ops)
        self.assertEqual(code, 1)
        self.assertEqual(result['stage'], 'backup')
        self.assertEqual(result['program_recovery'], 'old_programs_restored')
        self.assertEqual([call.args for call in signals.call_args_list], [
            (signal.SIGINT, signal.SIG_IGN), (signal.SIGTERM, signal.SIG_IGN), (signal.SIGHUP, signal.SIG_IGN)])

    def test_helpers_all_validated_before_any_is_executed(self):
        looked = []
        def source(path):
            looked.append(path.name)
            if path.name == 'snapshot_worker.py':
                raise ValueError('unsafe worker')
            return b'raise AssertionError("helper executed before full validation")'
        with patch.object(activation, 'trusted_source', side_effect=source):
            with self.assertRaises(ValueError):
                activation.trusted_modules()
        self.assertEqual(looked[-1], 'snapshot_worker.py')
        self.assertEqual(len(looked), 10)

    def test_cli_is_closed_and_does_not_echo_values(self):
        for args in ([], ['--manifest-sha256', 'secret'], ['--manifest', '0' * 64],
                     ['--manifest-sha256', '0' * 64, '--unsafe', 'secret']):
            with self.subTest(args=args), self.assertRaises(ValueError) as caught:
                activation.options(args, c)
            self.assertNotIn('secret', str(caught.exception))
        self.assertEqual(activation.options(['--manifest-sha256', '0' * 64], c).manifest_sha256, '0' * 64)


