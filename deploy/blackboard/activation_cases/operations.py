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
        # 停服务的命令时限按人侧单元的停止超时再加余量算（留出 systemd 到点强杀后的收尾），不另写一个数：
        # 命令时限比停止超时短的话，慢关停会让激活和回滚都在这条命令上超时。
        self.assertEqual([call.args[1] for call in ops.runtime.command.call_args_list],
                         [assembly.STOP_TIMEOUT_SECONDS + ops_module.STOP_COMMAND_MARGIN_SECONDS] * 2)
        # 余量和算出来的时限也钉住：两份 README 写了「加 20 秒」「320 秒」，改数要连文档一起改。
        self.assertEqual(ops_module.STOP_COMMAND_MARGIN_SECONDS, 20)
        self.assertEqual(assembly.STOP_TIMEOUT_SECONDS + ops_module.STOP_COMMAND_MARGIN_SECONDS, 320)

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
                patch.object(ops, 'backup_ready') as backup, \
                patch.object(ops, 'units', return_value={'studiolo-human': b'wrong', 'studiolo-agent': b'wrong'}):
            with self.assertRaises(ValueError):
                ops.preflight()
        backup.assert_called_once_with()
        ops.runtime.command.assert_not_called()

    def test_preflight_stops_at_missing_backup_prerequisites_before_any_service_command(self):
        # 备份前置没备好：核完候选包就停，不读现场单元、不碰任何服务命令（更谈不上停服）。
        ops = self.operation()
        with patch.object(ops_module.sys, 'platform', 'linux'), patch.object(os, 'getuid', return_value=0), \
                patch.object(os, 'geteuid', return_value=0), patch.object(ops_module.pwd, 'getpwnam',
                return_value=types.SimpleNamespace(pw_uid=1000)), \
                patch.object(ops_module, 'verify_candidate', return_value={}) as candidate, \
                patch.object(ops, 'backup_ready', side_effect=ValueError('rejected')) as backup, \
                patch.object(ops, 'units') as units:
            with self.assertRaises(ValueError):
                ops.preflight()
        candidate.assert_called_once()
        backup.assert_called_once_with()
        units.assert_not_called()
        ops.runtime.command.assert_not_called()

    def test_preflight_runs_the_candidate_launcher_against_the_live_sockets_before_any_stop(self):
        """BUG-058：候选启动器在现场不放行，预检就停下，不写回执、不停服。"""
        ops = self.operation()
        units = {'studiolo-human': b'old human', 'studiolo-agent': b'ExecStart=/agent/bin/studiolo-agent serve'}
        ops.old_shas = {name: c.sha(raw) for name, raw in units.items()}
        manifest = {'files': {'bin/studiolo-local-server': {'sha256': '2' * 64}}}
        stored = {Path(assembly.RELEASE) / 'studiolo-human.service': b'new human',
                  Path(assembly.RELEASE) / 'studiolo-agent.service': units['studiolo-agent'],
                  Path(assembly.OLD_RELEASE) / 'launch-human.py': b'old launcher',
                  Path(assembly.RELEASE) / 'launch-human.py': b'new launcher',
                  Path(assembly.OLD_RELEASE) / 'manifest.json': json.dumps(manifest).encode(),
                  ops_module.CONFIG: json.dumps({'seed_ref': 'studiolo-human-totp-v2', 'auth_directory': ops_module.AUTH,
                                                 'audit_directory': ops_module.AUDIT}).encode()}
        events = []
        ops.a = types.SimpleNamespace(OLD_RELEASE=assembly.OLD_RELEASE, RELEASE=assembly.RELEASE,
                                      R8_MANIFEST_SHA=c.sha(json.dumps(manifest).encode()),
                                      human_unit=lambda text: text.replace('old', 'new'), agent_unit=lambda text: text,
                                      launcher=lambda text: text.replace('old', 'new'))
        ops.runtime.ready.side_effect = lambda *args: events.append('human-ready')

        def run(accepts):
            events.clear()

            def probe(code):
                events.append(('launcher', code))
                if not accepts:
                    raise ValueError('rejected')
            with patch.object(ops_module.sys, 'platform', 'linux'), patch.object(os, 'getuid', return_value=0), \
                    patch.object(os, 'geteuid', return_value=0), patch.object(c, 'UID', 1000), \
                    patch.object(ops_module.pwd, 'getpwnam', return_value=types.SimpleNamespace(pw_uid=1000)), \
                    patch.object(ops_module, 'verify_candidate', return_value=manifest), \
                    patch.object(ops, 'backup_ready'), patch.object(ops, 'units', return_value=units), \
                    patch.object(c, 'read', side_effect=lambda path, *rest: stored[path]), \
                    patch.object(c, 'inspect'), patch.object(ops, 'worker', return_value={}), \
                    patch.object(ops_module, 'binding', return_value={}), \
                    patch.object(ops, 'board_ready', side_effect=lambda: events.append('board-ready')), \
                    patch.object(ops, 'read_agent', return_value=b'agent'), patch.object(ops, 'properties'), \
                    patch.object(ops, 'agent_ready', side_effect=lambda: events.append('agent-ready')), \
                    patch.object(ops, 'launcher_accepts', side_effect=probe), \
                    patch.object(ops, 'receipt', side_effect=lambda name, value: events.append(name)):
                ops.preflight()
        run(True)
        # 候选那一份的字节，人侧两个入口都核过是活的之后才跑，跑过才写回执。
        self.assertEqual(events, ['board-ready', 'human-ready', 'agent-ready', ('launcher', b'new launcher'), 'preflight'])
        ops.runtime.command.reset_mock()
        with self.assertRaises(ValueError):
            run(False)
        self.assertEqual(events[-1], ('launcher', b'new launcher'))
        # 停、起、重载一个都没有，只有只读的单元核验。
        self.assertEqual([call.args[0][0] for call in ops.runtime.command.call_args_list], ['systemd-analyze'])

    def test_launcher_probe_is_unprivileged_isolated_and_needs_both_live_sockets(self):
        ops = self.operation()
        ops.runtime.SOCKET = Path('/run/studiolo-human/http.sock')
        account = types.SimpleNamespace(pw_uid=1000, pw_gid=1000)

        run = Mock()

        def probe(*, returncode=0, missing=None, regular=None):
            def lstat(path):
                if str(path) == missing:
                    raise FileNotFoundError(path)
                return types.SimpleNamespace(st_mode=ops_module.stat.S_IFREG if str(path) == regular
                                             else ops_module.stat.S_IFSOCK)
            run.reset_mock()
            run.return_value = types.SimpleNamespace(returncode=returncode)
            with patch.object(c, 'UID', 1000), patch.object(ops_module.pwd, 'getpwnam', return_value=account), \
                    patch.object(os, 'lstat', side_effect=lstat), patch.object(ops_module.subprocess, 'run', run):
                ops.launcher_accepts(b'candidate bytes')
        probe()
        call = run.call_args
        self.assertEqual(call.args[0], ['/usr/bin/python3', '-I', '-B', '-S', '-c', ops_module.LAUNCHER_PROBE])
        self.assertEqual(call.kwargs['input'], b'candidate bytes')
        self.assertEqual((call.kwargs['user'], call.kwargs['group'], call.kwargs['extra_groups']), (1000, 1000, []))
        self.assertEqual((call.kwargs['cwd'], call.kwargs['env'], call.kwargs['timeout']), ('/', ops.runtime.ENV, 20))
        with self.assertRaises(ValueError):
            probe(returncode=1)
        # 主服务账号的号码对不上：不跑。
        account.pw_uid = 1001
        with self.assertRaises(ValueError):
            probe()
        run.assert_not_called()
        account.pw_uid = 1000
        # 两个入口少一个、或者不是 socket：这一步证明不了什么，拒绝，不跑。
        for path in ('/run/studiolo-human/http.sock', assembly.BOARD_SOCKET):
            with self.subTest(path=path):
                with self.assertRaises(FileNotFoundError):
                    probe(missing=path)
                run.assert_not_called()
                with self.assertRaises(ValueError):
                    probe(regular=path)
                run.assert_not_called()
        self.assertEqual(assembly.HUMAN_SOCKET, str(runtime.SOCKET))

    def test_launcher_probe_runs_preflight_only_and_reports_its_verdict(self):
        """真的跑一遍那段探测代码：只跑 preflight，不跑 main；preflight 抛错就是非零退出。"""
        def run(body):
            code = ('def preflight():\n    ' + body + '\n\n\nif __name__ == "__main__":\n    raise SystemExit(0)\n'
                    'if __name__ != "launcher_probe":\n    raise SystemExit(9)\n').encode()
            return subprocess.run([sys.executable, '-I', '-B', '-S', '-c', ops_module.LAUNCHER_PROBE], input=code,
                                  capture_output=True, timeout=30)
        accepted = run('print("ran")')
        self.assertEqual((accepted.returncode, accepted.stdout), (0, b'ran\n'))
        self.assertNotEqual(run('raise RuntimeError("rejected")').returncode, 0)

    def test_private_tmp_names_the_directories_the_unit_hides(self):
        # BackupReadyTests 把这个值换成沙盒里的目录来测逻辑；真实取值在这里钉。
        self.assertEqual(ops_module.PRIVATE_TMP, (Path('/tmp'), Path('/var/tmp')))
        # 依据：人侧单元 PrivateTmp=yes（看 169 现装的原文，装配不动这一行）。
        unit = (HERE / 'activated/studiolo-human.service').read_text()
        self.assertEqual(unit.count('PrivateTmp=yes\n'), 1)

    def test_backup_document_keys_and_floors_match_the_program(self):
        """预检核 backup.json 的键名和两个下限：对着程序解析它的源码钉，程序改了这里先红。"""
        def fields(relative):
            text = (HERE.parent.parent / relative).read_text()
            # 两个结构都是「多一个少一个键就拒绝」，预检才敢按「恰好这些键」核。
            # 结构体上只有派生和「多一个键就拒绝」两行属性，再上面是空行：多挂一行（比如 rename_all、default）这里先红。
            head = '\n\n#[derive(Deserialize)]\n#[serde(deny_unknown_fields)]\nstruct Document {\n'
            self.assertEqual(text.count(head), 1, relative)
            self.assertEqual(text.count('struct Document {'), 1, relative)
            body = text.split(head, 1)[1].split('\n}\n', 1)[0]
            # 字段名就是 JSON 键名、每个键都必填，靠的是字段上没有会改键名或让键可缺的属性
            # （rename、alias、default、flatten，或者包在 cfg_attr 里的）。字段上只许有一种属性：两段各自的解码函数，
            # 整行照这个样子写；别的属性、换了写法的都先红。
            for line in body.splitlines():
                if line.lstrip().startswith('#'):
                    self.assertRegex(line.strip(), r'^#\[serde\(deserialize_with = "[\w:]+"\)\]$', relative)
            return {name for name in re.findall(r'^    (\w+):', body, re.MULTILINE)}
        self.assertEqual(fields('crates/host-config/src/backup/file.rs'), ops_module.BACKUP_DOCUMENT_KEYS)
        self.assertEqual(fields('crates/host-config/src/backup/producer.rs'), ops_module.BACKUP_PRODUCER_KEYS)
        producer = (HERE.parent.parent / 'crates/host-config/src/backup/producer.rs').read_text()
        freeze = re.findall(r'^const FREEZE_MILLIS: std::ops::RangeInclusive<u32> = ([\d_]+)\.\.=([\d_]+);$',
                            producer, re.MULTILINE)
        self.assertEqual(len(freeze), 1)
        floor, ceiling = (int(value.replace('_', '')) for value in freeze[0])
        self.assertEqual(floor, ops_module.FREEZE_BUDGET_FLOOR_MILLIS)
        # 装配假设的冻结预算要落在程序接受的范围里，不然「全备好」的现场程序也不收。
        self.assertTrue(floor <= assembly.BACKUP_FREEZE_BUDGET_SECONDS * 1000 <= ceiling)
        validate = (HERE.parent.parent / 'crates/host-config/src/sync_transport/document/validate.rs').read_text()
        grace = re.findall(r'\((\d+)\.\.=(\d+)\)\.contains\(&document\.stop_grace_seconds\)', validate)
        self.assertEqual(len(grace), 1)
        floor, ceiling = (int(value) for value in grace[0])
        self.assertEqual(floor, ops_module.STOP_GRACE_FLOOR_SECONDS)
        self.assertTrue(floor <= assembly.BRIDGE_STOP_GRACE_SECONDS <= ceiling)

    def test_owner_groups_come_from_the_account_database(self):
        ops = self.operation()
        account = types.SimpleNamespace(pw_name='maxsong', pw_gid=1000)
        with patch.object(ops_module.pwd, 'getpwnam', return_value=account) as lookup, \
                patch.object(os, 'getgrouplist', return_value=[1000, 27, 994]) as groups:
            self.assertEqual(ops.owner_groups(), {1000, 27, 994})
        lookup.assert_called_once_with('maxsong')
        groups.assert_called_once_with('maxsong', 1000)

    def test_stopped_refuses_lingering_socket_without_removing_it(self):
        ops = self.operation()
        with patch.object(ops, 'properties', return_value={'ActiveState': 'inactive', 'MainPID': '0'}), \
                patch.object(os.path, 'lexists', return_value=True), patch.object(os, 'unlink') as unlink:
            with self.assertRaises(ValueError):
                ops.stopped()
        unlink.assert_not_called()
        # 三个入口各自留下都拒绝：人侧 HTTP、黑板、备份监听（r17 起人侧在监听它，正常关停会删掉）。
        for lingering in (ops.runtime.SOCKET, assembly.BOARD_SOCKET, assembly.BACKUP_SOCKET):
            with self.subTest(lingering=lingering), \
                    patch.object(ops, 'properties', return_value={'ActiveState': 'inactive', 'MainPID': '0'}), \
                    patch.object(os.path, 'lexists', side_effect=lambda path: path == lingering), \
                    patch.object(os, 'unlink') as unlink:
                with self.assertRaises(ValueError):
                    ops.stopped()
            unlink.assert_not_called()
        with patch.object(ops, 'properties', return_value={'ActiveState': 'inactive', 'MainPID': '0'}), \
                patch.object(os.path, 'lexists', return_value=False):
            ops.stopped()
        self.assertEqual(assembly.BACKUP_SOCKET, assembly.BACKUP_RUNTIME + '/producer.sock')


