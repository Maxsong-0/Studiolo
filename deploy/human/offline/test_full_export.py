#!/usr/bin/env python3
"""共同停写操作器的虚构失败注入；不调用真实systemctl或访问生产目录。"""
from contextlib import ExitStack
import importlib.util
from pathlib import Path
import stat
import subprocess
import types
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('full_export', Path(__file__).with_name('full_export.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class FreezeTests(unittest.TestCase):
    def fixture(self):
        stack = ExitStack()
        self.addCleanup(stack.close)
        self.calls = []
        self.run = stack.enter_context(patch.object(module, 'run', side_effect=lambda args, **kw: self.calls.append(list(args))))
        self.running = stack.enter_context(patch.object(module, 'check_running'))
        self.stores = stack.enter_context(patch.object(module, 'no_open_stores'))
        self.services = stack.enter_context(patch.object(module, 'service', return_value={'ActiveState': 'inactive'}))
        stack.enter_context(patch.object(module.os, 'open', return_value=99))
        stack.enter_context(patch.object(module.os, 'fstat', return_value=types.SimpleNamespace(st_uid=1000, st_mode=stat.S_IFREG | 0o600, st_nlink=1)))
        stack.enter_context(patch.object(module.os, 'close'))
        self.lock = stack.enter_context(patch.object(module.fcntl, 'flock'))
        return stack

    def system_calls(self):
        return [x for x in self.calls if x[0] == 'systemctl']

    def test_snapshot_failure_always_restarts_exact_original_units(self):
        self.fixture()
        def execute(args, **kwargs):
            self.calls.append(list(args))
            if 'create' in args:
                raise subprocess.CalledProcessError(1, 'fictional-snapshot')
        self.run.side_effect = execute
        with self.assertRaises(subprocess.CalledProcessError):
            module.freeze(Path('/fictional/case'), Path('/fictional/tools'))
        self.assertEqual(self.system_calls(), [
            ['systemctl', 'stop', 'studiolo-human.service', 'studiolo-agent.service'],
            ['systemctl', 'start', 'studiolo-agent.service', 'studiolo-human.service']])
        self.assertEqual(self.running.call_count, 2)

    def test_busy_replica_lock_does_not_stop_any_unit(self):
        self.fixture()
        self.lock.side_effect = BlockingIOError()
        with self.assertRaises(BlockingIOError):
            module.freeze(Path('/fictional/case'), Path('/fictional/tools'))
        self.assertEqual(self.system_calls(), [])

    def test_changed_running_binary_does_not_stop_any_unit(self):
        self.fixture()
        self.running.side_effect = ValueError('fictional-version-mismatch')
        with self.assertRaises(ValueError):
            module.freeze(Path('/fictional/case'), Path('/fictional/tools'))
        self.assertEqual(self.system_calls(), [])

    def test_unexpected_store_holder_restarts_original_units(self):
        self.fixture()
        self.stores.side_effect = ValueError('fictional-holder')
        with self.assertRaises(ValueError):
            module.freeze(Path('/fictional/case'), Path('/fictional/tools'))
        self.assertEqual(len(self.system_calls()), 2)
        self.assertEqual(self.system_calls()[-1][1], 'start')

    def test_restart_failure_is_not_reported_success(self):
        self.fixture()
        def execute(args, **kwargs):
            self.calls.append(list(args))
            if 'create' in args or 'start' in args:
                raise subprocess.CalledProcessError(1, 'fictional-failure')
        self.run.side_effect = execute
        with self.assertRaises(subprocess.CalledProcessError):
            module.freeze(Path('/fictional/case'), Path('/fictional/tools'))
        self.assertEqual(self.running.call_count, 1)

    def test_launcher_exec_delay_is_waited_without_another_restart(self):
        with patch.object(module, 'check_running', side_effect=[ValueError('launcher'), None]) as check, patch.object(module.time, 'sleep') as sleep:
            module.wait_running()
            self.assertEqual(check.call_count, 2)
            sleep.assert_called_once_with(0.05)

    def test_permanent_binary_mismatch_never_becomes_ready(self):
        with patch.object(module, 'check_running', side_effect=ValueError('mismatch')), patch.object(module.time, 'monotonic', side_effect=[0, 11]):
            with self.assertRaises(ValueError):
                module.wait_running()

    def test_capture_and_pipe_input_have_no_conflicting_arguments(self):
        with patch.object(module.subprocess, 'run') as run:
            module.run(['fictional'], capture_output=True)
            self.assertEqual(run.call_args.kwargs['stdout'], subprocess.PIPE)
            self.assertEqual(run.call_args.kwargs['stderr'], subprocess.DEVNULL)
            self.assertNotIn('capture_output', run.call_args.kwargs)
            module.run(['fictional'], stdin=77)
            self.assertEqual(run.call_args.kwargs['stdin'], 77)


if __name__ == '__main__':
    unittest.main()
