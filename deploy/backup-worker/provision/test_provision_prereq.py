"""prereq.py 整套流程的契约：对着一份内存里的「169 现场」预演、照做、重跑，再一样一样弄坏，看它该拦的拦、该放的放。

现场替身只在内存里：路径 → 节点（类型、属主、权限、内容），加账号表和组表；要 root 的命令由替身解释。
不碰真实的 /etc、/run 和账号库。由同目录的 test_provision.py 并进去一起跑（合并门禁只跑那一份），也可以单独跑。
"""
import contextlib
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile
import time
import types
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


prereq, nas = load('prereq'), load('nas_account')
OWNER, WORKER, READERS = prereq.OWNER, prereq.WORKER, prereq.READERS
HOME = '/home/' + OWNER
CANDIDATE = prereq.RELEASE_DIRECTORY + '/' + prereq.HUMAN_RELEASE + '/bin/'
WRONG, MISSING = '：已经在，但不是预期的样子。不覆盖，先由人看过', '：还没做好。先跑完 prereq.py --apply'


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


# 真实的摘要对应的是真实的程序；这里换成一组虚构内容和它们的摘要。
FICTION = {name: ('fictional ' + name).encode() for name in ('local-server', 'maintain', 'worker', 'restic', 'ca')}
PACKAGES = (('studiolo-local-server', digest(FICTION['local-server'])), ('studiolo-maintain', digest(FICTION['maintain'])),
            ('studiolo-backup-worker', digest(FICTION['worker'])))
UNIT_PATHS = [prereq.UNIT_DIRECTORY + '/' + name for name in prereq.UNITS]
GATED = ('studiolo-backup-worker.service', 'studiolo-backup-schedule.timer')


class World:
    """169 现场的内存替身。"""

    def __init__(self):
        self.nodes, self.accounts, self.groups, self.members = {}, {}, {}, {}
        self.ran, self.reloads, self.verify_fails, self.spare = [], 0, False, 990
        self.units = {name: {'is-enabled': 'disabled', 'is-active': 'inactive'} for name in prereq.UNITS}
        # swapped：读某个来源时给出的内容（模拟「核过之后被换了」）；before_place：放某个目标之前先发生的事。
        self.swapped, self.before_place, self.interrupt_at = {}, {}, None

    # ---- 搭现场
    def directory(self, path, uid=0, gid=0, mode=0o755):
        self.nodes[path] = types.SimpleNamespace(kind='dir', uid=uid, gid=gid, mode=mode, raw=None, links=1)

    def file(self, path, raw, uid=0, gid=0, mode=0o644):
        self.nodes[path] = types.SimpleNamespace(kind='file', uid=uid, gid=gid, mode=mode, raw=raw, links=1)

    def account(self, name, uid, gid, home=HOME, shell='/bin/bash'):
        self.accounts[name] = types.SimpleNamespace(pw_name=name, pw_uid=uid, pw_gid=gid, pw_dir=home, pw_shell=shell)

    def group(self, name, gid):
        self.groups[name] = types.SimpleNamespace(gr_name=name, gr_gid=gid)

    def snapshot(self):
        return ({path: (node.kind, node.uid, node.gid, node.mode, node.raw, node.links) for path, node in self.nodes.items()},
                {name: vars(entry).copy() for name, entry in self.accounts.items()},
                {name: entry.gr_gid for name, entry in self.groups.items()},
                {name: set(gids) for name, gids in self.members.items()})

    # ---- prereq.py 看现场用的那几个函数的替身
    def has(self, path, kind, uid, gid, modes):
        node = self.nodes.get(path)
        if node is None:
            return None
        wanted = {stat.S_ISREG: 'file', stat.S_ISDIR: 'dir'}[kind]
        return (node.kind == wanted and (node.uid, node.gid) == (uid, gid) and node.mode in modes
                and (node.kind == 'dir' or node.links == 1))

    def content(self, path):
        if path not in self.nodes:
            raise FileNotFoundError(path)
        return self.nodes[path].raw

    def sha(self, path):
        return digest(self.content(path))

    def matches(self, path, wanted):
        node = self.nodes.get(path)
        return node is not None and node.kind == 'file' and digest(node.raw) == wanted

    def reading(self, path):
        return io.BytesIO(self.swapped.get(path, self.content(path)))

    def make_directory(self, path, mode):
        if path in self.nodes:
            raise FileExistsError(path)
        self.directory(path, prereq.OWNER_UID, prereq.OWNER_UID, mode)

    def chain(self, path, uid):
        """和真的 chain 同一条规则，看的是内存里的节点；链接在这里记成 kind='link'。"""
        for parent in reversed(Path(path).parents):
            node = self.nodes.get(str(parent))
            if node is None:
                raise prereq.Refused(str(parent) + ' 不在')
            prereq.require(node.kind == 'dir' and node.uid in (0, uid) and node.mode & 0o7022 == 0,
                           str(parent) + ' 不是真目录（路径上不能有链接），或者属主不对、组和其他人可写')

    def group_numbered(self, gid):
        return next((entry for entry in self.groups.values() if entry.gr_gid == gid), None)

    def groups_of(self, name, gid):
        return [gid] + sorted(self.members.get(name, ()))

    def place(self, destination, mode, stream, owner, wanted):
        """和真的 place 同一个约定：已经在就失败；内容的摘要不是给的那个就不放。"""
        raw = stream.read()
        if destination in self.nodes:
            raise FileExistsError(destination)
        prereq.require(digest(raw) == wanted, destination + ' 要放的内容和核过的那一份不一样（来源在核对之后被换了）。什么都没放')
        uid, gid = owner if owner is not None else (prereq.OWNER_UID, prereq.OWNER_UID)
        self.file(destination, raw, uid, gid, mode)

    @contextlib.contextmanager
    def installed(self, uid=prereq.OWNER_UID, euid=prereq.OWNER_UID, platform='linux'):
        with contextlib.ExitStack() as stack:
            for name in ('has', 'sha', 'content', 'matches', 'reading', 'make_directory', 'chain', 'group_numbered',
                         'groups_of', 'place'):
                stack.enter_context(patch.object(prereq, name, getattr(self, name)))
            for name, value in (('account', self.accounts.get), ('group', self.groups.get),
                                ('system_range', lambda kind: (100, 999)), ('PACKAGES', PACKAGES),
                                ('RESTIC_SHA', digest(FICTION['restic'])), ('CA_SHA', digest(FICTION['ca'])),
                                ('sys', types.SimpleNamespace(platform=platform, stderr=sys.stderr))):
                stack.enter_context(patch.object(prereq, name, value))
            stack.enter_context(patch.object(prereq.os, 'getuid', return_value=uid))
            stack.enter_context(patch.object(prereq.os, 'geteuid', return_value=euid))
            yield self

    # ---- 命令的替身
    def run(self, argv, **options):
        self.ran.append(list(argv))
        done = types.SimpleNamespace(returncode=0, stdout='')
        if argv[:2] == ['sudo', '--']:
            assert options.get('check') is True, argv
            self.as_root(argv[2:], options)
        elif argv[:3] == ['systemd-analyze', 'verify', '--recursive-errors=no']:
            assert (argv[3:], options) == (UNIT_PATHS, {'check': True}), argv
            if self.verify_fails:
                raise subprocess.CalledProcessError(1, argv)
        elif argv[0] == 'systemctl' and argv[1] in ('is-enabled', 'is-active') and len(argv) == 3:
            assert options == {'capture_output': True, 'text': True}, options
            done.stdout = self.units[argv[2]][argv[1]] + '\n'
        else:
            raise AssertionError('没预料到的命令：' + ' '.join(argv))
        return done

    def as_root(self, argv, options):
        if argv == ['groupadd', '--system', READERS]:
            assert READERS not in self.groups
            self.group(READERS, self.next_number())
        elif argv == ['useradd', '--system', '--user-group', '--no-create-home', '--home-dir', prereq.WORKER_HOME,
                      '--shell', prereq.WORKER_SHELL, WORKER]:
            assert WORKER not in self.accounts and WORKER not in self.groups
            number = self.next_number()
            self.group(WORKER, number)
            self.account(WORKER, number, number, prereq.WORKER_HOME, prereq.WORKER_SHELL)
        elif argv[:2] == ['usermod', '-aG'] and len(argv) == 4:
            self.members.setdefault(argv[3], set()).add(self.groups[argv[2]].gr_gid)
        elif argv == ['systemd-tmpfiles', '--create', prereq.TMPFILES]:
            for line in self.content(prereq.TMPFILES).decode().splitlines():
                kind, path, mode, user, group, age = line.split()
                assert (kind, age) == ('d', '-'), line
                # 已经在的不动：走到这一步时，在的那个已经核过是对的（不对的话前一步就停下了）。
                if path not in self.nodes:
                    self.directory(path, self.accounts[user].pw_uid, self.groups[group].gr_gid, int(mode, 8))
        elif argv == ['mkdir', '-m', '0755', prereq.PROGRAMS]:
            if prereq.PROGRAMS in self.nodes:
                raise subprocess.CalledProcessError(1, argv)
            self.directory(prereq.PROGRAMS)
        elif argv == ['systemctl', 'daemon-reload']:
            if self.interrupt_at == 'daemon-reload':
                raise prereq.Interrupted('SIGHUP')
            self.reloads += 1
        elif argv[:5] == ['/usr/bin/python3', '-I', '-B', str(HERE / 'prereq.py'), '--place']:
            self.as_root_place(argv, options)
        else:
            raise AssertionError('没预料到的 sudo 命令：' + ' '.join(argv))

    def as_root_place(self, argv, options):
        """root 一侧走的是脚本自己真的 --place 入口（只换掉身份、标准输入和最后落盘的那一下）；
        它停下就是退出码 1，这一侧看到的是命令失败。"""
        self.before_place.pop(argv[5], lambda: None)()
        stream = io.BytesIO(options['input']) if 'input' in options else options['stdin']
        piped = types.SimpleNamespace(platform='linux', stdin=types.SimpleNamespace(isatty=lambda: False, buffer=stream))
        try:
            with patch.object(prereq.os, 'geteuid', return_value=0), patch.object(prereq, 'sys', piped):
                prereq.main(argv[4:])
        except (prereq.Refused, OSError) as failure:
            raise subprocess.CalledProcessError(1, ['sudo', '--'] + argv) from failure

    def next_number(self):
        self.spare -= 1
        return self.spare


def fresh():
    """还没置备过的 169：人侧在、候选在、要拷的三样在，备份 worker 的东西一样都没有。"""
    world = World()
    for path in ('/', '/etc', '/etc/studiolo', '/etc/tmpfiles.d', '/etc/systemd', '/etc/systemd/system', '/usr', '/usr/lib',
                 '/usr/local', '/usr/local/bin', '/run', '/var', '/var/lib', '/home'):
        world.directory(path)
    world.directory(HOME, prereq.OWNER_UID, prereq.OWNER_UID, 0o750)
    for path in ('.config', '.config/studiolo-human', '.config/studiolo-backup', '.local', '.local/share'):
        world.directory(HOME + '/' + path, prereq.OWNER_UID, prereq.OWNER_UID, 0o700)
    for path, name in ((prereq.WORKER_SOURCE, 'worker'), (prereq.RESTIC_SOURCE, 'restic'), (prereq.CA_SOURCE, 'ca'),
                       (CANDIDATE + 'studiolo-local-server', 'local-server'), (CANDIDATE + 'studiolo-maintain', 'maintain')):
        world.file(path, FICTION[name], prereq.OWNER_UID, prereq.OWNER_UID, 0o755)
    world.account(OWNER, prereq.OWNER_UID, prereq.OWNER_UID)
    world.group(OWNER, prereq.OWNER_UID)
    return world


def label(argv):
    """一条命令的短名字：只为了核先后顺序。"""
    if argv[:2] != ['sudo', '--']:
        return ' '.join(argv[:3]) if argv[0] == 'systemctl' else ' '.join(argv[:2])
    if '--place' in argv:
        return 'place ' + argv[argv.index('--place') + 1]
    return ' '.join(argv[2:4]) if argv[2] in ('usermod', 'systemctl') else argv[2]


FIRST_RUN = (['groupadd', 'useradd', 'usermod -aG', 'usermod -aG', 'place ' + prereq.TMPFILES, 'systemd-tmpfiles', 'mkdir',
              'place ' + prereq.WORKER_BINARY, 'place ' + prereq.RESTIC, 'place ' + prereq.CA, 'place ' + prereq.WORKER_CONFIG]
             + ['place ' + path for path in UNIT_PATHS])
GATE = (['systemctl daemon-reload', 'systemd-analyze verify']
        + ['systemctl ' + verb + ' ' + name for name in GATED for verb in ('is-enabled', 'is-active')])
STEPS = 16


class ProvisionTests(unittest.TestCase):
    def provision(self, world, apply=True):
        self.lines = []
        return prereq.Provision(apply, run=world.run, out=self.lines.append)

    def built(self):
        """置备完成的现场。"""
        world = fresh()
        with world.installed():
            self.assertEqual(self.provision(world).everything(), STEPS)
        world.ran.clear()
        return world

    def refused(self, world, **identity):
        """照做一遍，期望停下：给出拒绝的原话。停下之前一条命令都没跑，现场一样没动。"""
        before = world.snapshot()
        with world.installed(**identity), self.assertRaises(prereq.Refused) as refusal:
            self.provision(world).everything()
        self.assertEqual((world.ran, world.snapshot()), ([], before))
        return str(refusal.exception)

    def test_a_preview_changes_nothing_and_shows_every_step_and_both_documents(self):
        world = fresh()
        before = world.snapshot()
        with world.installed():
            pending = self.provision(world, apply=False).everything()
            worker = prereq.encode(prereq.worker_document(*prereq.PLACEHOLDER_NUMBERS)).decode()
            backup = prereq.encode(prereq.backup_document(*prereq.PLACEHOLDER_NUMBERS)).decode()
        self.assertEqual((pending, world.ran, world.snapshot()), (STEPS, [], before))
        self.assertEqual(len([line for line in self.lines if line.startswith('[要做] ')]), STEPS)
        self.assertFalse([line for line in self.lines if line.startswith(('[现在做] ', '[已有] ', '    核对通过'))])
        self.assertIn('（账号还没建：下面两份配置里的 64001、64002 是占位的号码，照做时换成真号码）', self.lines)
        self.assertEqual(self.lines[-1], '\n---- ' + prereq.WORKER_CONFIG + '\n' + worker + '---- ' + prereq.BACKUP_CONFIG
                         + '\n' + backup)
        self.assertIn('    sudo systemctl daemon-reload', self.lines)
        self.assertIn('    sudo useradd --system --user-group --no-create-home --home-dir /nonexistent'
                      ' --shell /usr/sbin/nologin studiolo-backup', self.lines)

    def test_apply_builds_exactly_the_expected_site_and_a_second_run_changes_nothing(self):
        world = fresh()
        before = world.snapshot()
        with world.installed():
            with self.assertRaises(prereq.Refused) as refusal:
                prereq.settled()
            self.assertEqual(str(refusal.exception), '读取组 ' + READERS + '（系统组）' + MISSING)
            self.assertEqual(world.snapshot(), before)
            self.assertEqual(self.provision(world).everything(), STEPS)
            worker, readers = world.accounts[WORKER], world.groups[READERS].gr_gid
            owned, private = (prereq.OWNER_UID, readers, 0o750, None, 1), (prereq.OWNER_UID, prereq.OWNER_UID)
            expected = {
                prereq.TMPFILES: ('file', 0, 0, 0o644, prereq.tmpfiles_text().encode(), 1),
                prereq.RUNTIME: ('dir',) + owned, prereq.STAGING: ('dir',) + owned,
                prereq.PROGRAMS: ('dir', 0, 0, 0o755, None, 1),
                prereq.WORKER_BINARY: ('file', 0, 0, 0o755, FICTION['worker'], 1),
                prereq.RESTIC: ('file', 0, 0, 0o555, FICTION['restic'], 1),
                prereq.CA: ('file', 0, 0, 0o644, FICTION['ca'], 1),
                prereq.WORKER_CONFIG: ('file', 0, 0, 0o644, prereq.encode(prereq.worker_document(worker.pw_uid, readers)), 1),
                prereq.EPHEMERAL: ('dir',) + private + (0o700, None, 1),
                prereq.BACKUP_CONFIG: ('file',) + private + (
                    0o600, prereq.encode(prereq.backup_document(worker.pw_uid, readers)), 1),
            }
            expected.update({path: ('file', 0, 0, 0o644, (prereq.TEMPLATES / Path(path).name).read_bytes(), 1)
                             for path in UNIT_PATHS})
            nodes, accounts, groups, members = world.snapshot()
            # 多出来的正好是这些，原来就有的一样没动。
            self.assertEqual({path: node for path, node in nodes.items() if path not in before[0]}, expected)
            self.assertEqual({path: nodes[path] for path in before[0]}, before[0])
            self.assertEqual(sorted(accounts), sorted([OWNER, WORKER]))
            self.assertEqual((worker.pw_dir, worker.pw_shell, groups[WORKER]), ('/nonexistent', '/usr/sbin/nologin', worker.pw_gid))
            self.assertEqual(members, {WORKER: {readers}, OWNER: {readers}})
            # 先后：账号和组 → 目录 → 程序和证书 → worker 配置 → 单元 → 重新加载、核语法、看没启用没在跑。
            self.assertEqual([label(argv) for argv in world.ran], FIRST_RUN + GATE)
            self.assertEqual((world.reloads, self.lines[-1]), (1, '    三个单元语法核过，都没启用、没在跑'))
            self.assertEqual(len([line for line in self.lines if line == '    核对通过']), STEPS)
            # 第二遍：一步都不用做，现场不变；重新加载和那道闸照样走一遍（上一次要是停在放完单元、还没重新加载，这里补上）。
            world.ran.clear()
            built = world.snapshot()
            self.assertEqual(self.provision(world).everything(), 0)
            self.assertEqual((world.snapshot(), [label(argv) for argv in world.ran], world.reloads), (built, GATE, 2))
            self.assertEqual(len([line for line in self.lines if line.startswith('[已有] ')]), STEPS)
            # 只读地核全部产物：一样不少，什么命令都不跑。
            world.ran.clear()
            said = []
            self.assertEqual(prereq.settled(said.append), STEPS)
            self.assertEqual((world.snapshot(), world.ran, len(said)), (built, [], STEPS))
            # 置备之后的预演：还差 0 步。
            self.assertEqual(self.provision(world, apply=False).everything(), 0)
            self.assertEqual((world.snapshot(), world.ran), (built, []))

    def test_it_only_runs_on_the_production_host_as_the_owner_and_never_under_sudo(self):
        told = '要在 169 上以 ' + OWNER + ' 跑，不要用 sudo 起这个脚本'
        for identity in ({'platform': 'darwin'}, {'uid': 1001, 'euid': 1001}, {'euid': 0}, {'uid': 0, 'euid': 0},
                         {'uid': 0}):
            with self.subTest(identity):
                self.assertEqual(self.refused(fresh(), **identity), told)
        world = fresh()
        del world.accounts[OWNER]
        self.assertEqual(self.refused(world), told)
        world = fresh()
        world.account(OWNER, 1001, 1001)
        self.assertEqual(self.refused(world), told)

    def test_every_source_and_both_candidate_programs_must_be_there_with_the_registered_digest(self):
        sources = {prereq.WORKER_SOURCE: 'worker 程序 的来源 ', prereq.RESTIC_SOURCE: 'restic 的来源 ',
                   prereq.CA_SOURCE: 'NAS 证书 的来源 '}
        candidates = {CANDIDATE + name: '候选里的 ' + name + ' 不在或摘要和这里登记的不一样' for name, _ in PACKAGES[:2]}
        for path in list(sources) + list(candidates):
            told = sources[path] + path + ' 不在或摘要不对' if path in sources else candidates[path]
            for kind in ('不在', '内容不同', '是个目录'):
                with self.subTest(path=path, kind=kind):
                    world = fresh()
                    if kind == '不在':
                        del world.nodes[path]
                    elif kind == '内容不同':
                        world.nodes[path].raw += b'!'
                    else:
                        world.directory(path)
                    self.assertEqual(self.refused(world), told)

    def test_the_ground_everything_lands_on_is_checked_before_anything_is_built(self):
        root, config = '/etc/studiolo 不是 root 的 0755 目录', '人侧私有配置目录不是 ' + OWNER + ' 的 0700 目录'
        loose = ' 不是真目录（路径上不能有链接），或者属主不对、组和其他人可写'
        private = HOME + '/.config/studiolo-human'
        cases = (
            ('/etc/studiolo', {'mode': 0o775}, root), ('/etc/studiolo', {'uid': prereq.OWNER_UID}, root),
            ('/etc/studiolo', {'gid': 4}, root), ('/etc/studiolo', {'kind': 'file'}, root), ('/etc/studiolo', None, root),
            (private, {'mode': 0o750}, config), (private, {'uid': 0}, config), (private, {'gid': 4}, config),
            (private, None, config),
            # 目录链：每个要建的东西上面各有一级只属于它自己的目录，逐个弄坏。
            (HOME + '/.config', {'mode': 0o770}, HOME + '/.config' + loose),        # 私有备份配置
            (HOME + '/.local', {'mode': 0o707}, HOME + '/.local' + loose),          # 临时数据根
            ('/usr/lib', {'uid': prereq.OWNER_UID}, '/usr/lib' + loose),            # 程序目录
            ('/etc/tmpfiles.d', {'mode': 0o775}, '/etc/tmpfiles.d' + loose),        # tmpfiles 片段
            ('/etc/systemd', {'kind': 'link'}, '/etc/systemd' + loose),             # 单元
            ('/run', {'mode': 0o1777}, '/run' + loose),                             # socket 的父目录
            ('/var/lib', None, '/var/lib 不在'),                                    # 封存父目录
            ('/etc', {'mode': 0o757}, '/etc' + loose),                              # worker 配置（和上面几样共用）
            ('/home', {'uid': 1001}, '/home' + loose), ('/', {'mode': 0o777}, '/' + loose),
        )
        for path, change, told in cases:
            with self.subTest(path=path, change=change):
                world = fresh()
                if change is None:
                    del world.nodes[path]
                else:
                    vars(world.nodes[path]).update(change)
                self.assertEqual(self.refused(world), told)
                # 只读核对那一路（credentials.py 用）同样先看落脚的地方。
                with world.installed(), self.assertRaises(prereq.Refused) as refusal:
                    prereq.settled()
                self.assertEqual(str(refusal.exception), told)
        # 主服务账号自己的目录链上，属主是它自己或 root 都行；root 的那几条链上只认 root。
        world = fresh()
        world.nodes[HOME + '/.local'].uid = 0
        with world.installed():
            self.assertEqual(self.provision(world).everything(), STEPS)

    def test_an_existing_group_or_account_must_look_like_the_one_this_script_would_create(self):
        readers = '读取组 ' + READERS + '（系统组）'
        account = '专用账号 ' + WORKER + '（系统账号，不能登录）'

        def site(change, readers_gid=980, uid=985, gid=985, group_name=WORKER, group_gid=985):
            world = fresh()
            world.group(READERS, readers_gid)
            if uid is not None:
                world.account(WORKER, uid, gid, prereq.WORKER_HOME, prereq.WORKER_SHELL)
                vars(world.accounts[WORKER]).update(change)
                if group_name:
                    world.group(group_name, group_gid)
            return world
        # 读取组得是系统组：普通组占了这个名字不算。范围的两头放过。
        for number in (99, 1000, 1500):
            with self.subTest(readers=number):
                self.assertEqual(self.refused(site({}, readers_gid=number, uid=None)), readers + WRONG)
        refusals = {
            'root': ({}, {'uid': 0}), '就是主服务账号': ({}, {'uid': prereq.OWNER_UID}),
            '普通账号': ({}, {'uid': 1500}), '号码在系统范围下面': ({}, {'uid': 99}),
            '主组不同名': ({}, {'group_name': 'users'}), '主组查不到': ({}, {'group_name': None}),
            '主组是普通组': ({}, {'gid': 1600, 'group_gid': 1600}),
            '能登录': ({'pw_shell': '/bin/bash'}, {}), 'shell 只是名字像': ({'pw_shell': '/usr/sbin/nologin-not'}, {}),
            '有家目录': ({'pw_dir': '/home/' + WORKER}, {}), '家目录是根': ({'pw_dir': '/'}, {}),
        }
        for name, (change, numbers) in refusals.items():
            with self.subTest(name):
                self.assertEqual(self.refused(site(change, **numbers)), account + WRONG)
        # 和新建出来的一个样的，接着用：不再建账号和组，只补上读取组的成员关系。
        for name, (change, numbers) in {'一样': ({}, {}), '另一个 nologin': ({'pw_shell': '/sbin/nologin'}, {}),
                                        '范围最小': ({}, {'uid': 100, 'gid': 100, 'group_gid': 100, 'readers_gid': 999}),
                                        '范围最大': ({}, {'uid': 999, 'gid': 999, 'group_gid': 999, 'readers_gid': 100})}.items():
            with self.subTest(name):
                world = site(change, **numbers)
                with world.installed():
                    self.assertEqual(self.provision(world).everything(), STEPS - 2)
                self.assertEqual([label(argv) for argv in world.ran], FIRST_RUN[2:] + GATE)
                self.assertEqual(world.members, {WORKER: {world.groups[READERS].gr_gid}, OWNER: {world.groups[READERS].gr_gid}})
        # 已经在读取组里的不再加；主组就是读取组的也算在里面。
        world = site({})
        world.members[WORKER] = {980}
        world.accounts[OWNER].pw_gid = 980
        with world.installed():
            self.assertEqual(self.provision(world).everything(), STEPS - 4)
        self.assertEqual([label(argv) for argv in world.ran], FIRST_RUN[4:] + GATE)

    def test_a_product_that_is_there_but_wrong_is_never_overwritten_and_a_missing_one_is_rebuilt(self):
        """置备好的现场逐样弄坏：照做的那一路停下、不覆盖；只读核对的那一路（credentials.py 用）同样停下。
        弄没的：照做会补上，只读核对会说还没做好。"""
        titles = {
            prereq.TMPFILES: 'tmpfiles 片段 ' + prereq.TMPFILES,
            prereq.RUNTIME: '两个目录 ' + prereq.RUNTIME + '、' + prereq.STAGING + '（' + OWNER + ':' + READERS + ' 0750）',
            prereq.PROGRAMS: '程序目录 ' + prereq.PROGRAMS, prereq.WORKER_BINARY: 'worker 程序',
            prereq.RESTIC: 'restic 0.19.1', prereq.CA: 'NAS 证书 ' + prereq.CA,
            prereq.WORKER_CONFIG: 'worker 配置 ' + prereq.WORKER_CONFIG,
            prereq.EPHEMERAL: '临时数据根 ' + prereq.EPHEMERAL + '（' + OWNER + ' 私有 0700）',
            prereq.BACKUP_CONFIG: '私有备份配置 ' + prereq.BACKUP_CONFIG + '（不用 sudo，0600）',
        }
        titles[prereq.STAGING] = titles[prereq.RUNTIME]
        titles.update({path: '单元 ' + Path(path).name for path in UNIT_PATHS})
        files = [path for path in titles if path not in (prereq.RUNTIME, prereq.STAGING, prereq.PROGRAMS, prereq.EPHEMERAL)]
        wrong = [(path, {'raw': b'other content\n'}) for path in files]
        wrong += [(path, {'mode': 0o600}) for path in files if path != prereq.BACKUP_CONFIG]
        wrong += [(path, {'uid': 12}) for path in titles] + [(path, {'gid': 12}) for path in titles]
        wrong += [(path, {'links': 2}) for path in files]
        wrong += [(prereq.BACKUP_CONFIG, {'mode': 0o644}), (prereq.RESTIC, {'mode': 0o755}), (prereq.WORKER_BINARY, {'mode': 0o555}),
                  (prereq.RUNTIME, {'mode': 0o755}), (prereq.STAGING, {'mode': 0o770}), (prereq.PROGRAMS, {'mode': 0o775}),
                  (prereq.EPHEMERAL, {'mode': 0o750}), (prereq.CA, {'kind': 'dir'}), (prereq.PROGRAMS, {'kind': 'file'})]
        self.assertEqual(len(titles), 13)
        for path, change in wrong:
            with self.subTest(path=path, change=sorted(change)):
                world = self.built()
                vars(world.nodes[path]).update(change)
                broken = world.snapshot()
                with world.installed():
                    for check in (lambda: self.provision(world).everything(), prereq.settled):
                        with self.assertRaises(prereq.Refused) as refusal:
                            check()
                        self.assertEqual(str(refusal.exception), titles[path] + WRONG)
                # 不对的那一样原样留着；要 root 的命令一条都没跑。
                self.assertEqual((world.snapshot(), world.ran), (broken, []))
        for path in titles:
            with self.subTest(missing=path):
                world = self.built()
                complete = world.snapshot()
                del world.nodes[path]
                with world.installed():
                    with self.assertRaises(prereq.Refused) as refusal:
                        prereq.settled()
                    self.assertEqual(str(refusal.exception), titles[path] + MISSING)
                    if path == prereq.PROGRAMS:
                        continue  # 目录没了、里面的文件还在，这种现场造不出来：只看只读核对那一侧
                    self.assertEqual(self.provision(world).everything(), 1)
                    self.assertEqual(prereq.settled(), STEPS)
                self.assertEqual(world.snapshot(), complete)

    def test_identities_that_drift_after_provisioning_are_caught_by_both_paths(self):
        account = '专用账号 ' + WORKER + '（系统账号，不能登录）'
        world = self.built()
        world.accounts[WORKER].pw_shell = '/bin/sh'
        with world.installed():
            for check in (lambda: self.provision(world).everything(), prereq.settled):
                with self.assertRaises(prereq.Refused) as refusal:
                    check()
                self.assertEqual(str(refusal.exception), account + WRONG)
        for name in (WORKER, OWNER):
            with self.subTest(name):
                world = self.built()
                world.members[name].clear()
                with world.installed():
                    with self.assertRaises(prereq.Refused) as refusal:
                        prereq.settled()
                    self.assertEqual(str(refusal.exception), name + ' 加进读取组' + MISSING)
                    self.assertEqual(self.provision(world).everything(), 1)
                    self.assertEqual(prereq.settled(), STEPS)
        # 账号重建过、号码变了：两份配置里写的还是旧号码，内容对不上。
        world = self.built()
        world.accounts[WORKER].pw_uid += 1
        with world.installed(), self.assertRaises(prereq.Refused) as refusal:
            prereq.settled()
        self.assertEqual(str(refusal.exception), 'worker 配置 ' + prereq.WORKER_CONFIG + WRONG)

    def test_units_must_verify_and_must_not_be_enabled_or_running(self):
        world = fresh()
        world.verify_fails = True
        with world.installed(), self.assertRaises(subprocess.CalledProcessError):
            self.provision(world).everything()
        self.assertEqual([label(argv) for argv in world.ran], FIRST_RUN + GATE[:2])
        for name in GATED:
            for verb, answer in (('is-enabled', 'enabled'), ('is-active', 'active'), ('is-enabled', 'enabled-runtime'),
                                 ('is-active', 'activating'), ('is-active', 'failed'), ('is-enabled', '')):
                with self.subTest(name=name, answer=answer):
                    world = self.built()
                    world.units[name][verb] = answer
                    with world.installed():
                        with self.assertRaises(prereq.Refused) as refusal:
                            self.provision(world).everything()
                        self.assertEqual(str(refusal.exception), name + ' 不该是启用或在跑的')
                        self.assertNotIn('    三个单元语法核过，都没启用、没在跑', self.lines)
                        # 只读核对不看这一条（置备当时的条件）；预演也不问 systemd。
                        world.ran.clear()
                        self.assertEqual(prereq.settled(), STEPS)
                        self.assertEqual(self.provision(world, apply=False).everything(), 0)
                        self.assertEqual(world.ran, [])
        # 由定时器带起来的那个服务单元不在这道闸里（它平时就是 static、间歇地跑）。
        world = self.built()
        world.units['studiolo-backup-schedule.service'] = {'is-enabled': 'static', 'is-active': 'active'}
        with world.installed():
            self.assertEqual(self.provision(world).everything(), 0)

    def test_a_run_cut_off_between_placing_the_units_and_reloading_is_completed_by_the_next_run(self):
        world = fresh()
        world.interrupt_at = 'daemon-reload'
        with world.installed(), self.assertRaises(KeyboardInterrupt):
            self.provision(world).everything()
        self.assertEqual((world.reloads, [label(argv) for argv in world.ran]), (0, FIRST_RUN + GATE[:1]))
        world.interrupt_at = None
        world.ran.clear()
        with world.installed():
            self.assertEqual(self.provision(world).everything(), 0)
        self.assertEqual((world.reloads, [label(argv) for argv in world.ran]), (1, GATE))

    def test_a_target_that_appears_or_a_source_that_changes_after_the_check_is_not_overwritten_or_installed(self):
        # 从「看过不在」到真正去放之间，目标被别人建了：失败，别人的那一份原样留着。
        world = fresh()
        world.before_place[prereq.CA] = lambda: world.file(prereq.CA, b'somebody else', 0, 0, 0o644)
        with world.installed(), self.assertRaises(subprocess.CalledProcessError) as failure:
            self.provision(world).everything()
        self.assertIsInstance(failure.exception.__cause__, FileExistsError)
        self.assertEqual(world.content(prereq.CA), b'somebody else')
        self.assertEqual(label(world.ran[-1]), 'place ' + prereq.CA)
        # 来源在核过摘要之后被换了：root 一侧放之前再核一遍，不放。
        world = fresh()
        world.swapped[prereq.RESTIC_SOURCE] = b'swapped after the check'
        with world.installed():
            with self.assertRaises(subprocess.CalledProcessError) as failure:
                self.provision(world).everything()
            self.assertIn('要放的内容和核过的那一份不一样', str(failure.exception.__cause__))
            self.assertNotIn(prereq.RESTIC, world.nodes)
            self.assertEqual(label(world.ran[-1]), 'place ' + prereq.RESTIC)
            # 换回来之后重跑，从停下的地方接着做完。
            world.swapped.clear()
            self.assertEqual(self.provision(world).everything(), STEPS - 8)
            self.assertEqual(prereq.settled(), STEPS)


class LookupTests(unittest.TestCase):
    """看现场用的几个小函数（上面的流程用例把它们换成了替身，这里核它们自己）。"""

    def test_matches_needs_a_regular_file_with_the_given_digest(self):
        with tempfile.TemporaryDirectory() as scratch:
            path = os.path.join(scratch, 'file')
            self.assertFalse(prereq.matches(path, digest(b'')))
            Path(path).write_bytes(b'content')
            self.assertTrue(prereq.matches(path, digest(b'content')))
            self.assertFalse(prereq.matches(path, digest(b'other')))
            self.assertFalse(prereq.matches(scratch, digest(b'')))
            self.assertEqual((prereq.sha(path), prereq.content(path)), (digest(b'content'), b'content'))
            with prereq.reading(path) as stream:
                self.assertEqual(stream.read(), b'content')
            prereq.make_directory(path + '.d', 0o700)
            self.assertEqual(stat.S_IMODE(os.lstat(path + '.d').st_mode), 0o700)
            with self.assertRaises(FileExistsError):
                prereq.make_directory(path + '.d', 0o700)

    def test_system_range_reads_the_same_keys_useradd_does(self):
        with tempfile.TemporaryDirectory() as scratch:
            path = os.path.join(scratch, 'login.defs')
            cases = (
                ('SYS_UID_MIN 201\nSYS_UID_MAX\t899\nUID_MIN 1000\nSYS_GID_MIN   100 # 注释\nSYS_GID_MAX 999\n',
                 (201, 899), (100, 999)),
                # 没写系统范围：从 101 到普通账号起始号的前一个。
                ('UID_MIN 500\nGID_MIN 600\n', (101, 499), (101, 599)),
                # 注释掉的、值不是数字的、多一个词的，都不算。
                ('#SYS_UID_MIN 5\nSYS_UID_MAX abc\nSYS_GID_MIN 7 8\n', (101, 999), (101, 999)),
                ('', (101, 999), (101, 999)),
            )
            for text, users, groups in cases:
                with self.subTest(text[:20]):
                    Path(path).write_text(text, encoding='utf-8')
                    self.assertEqual((prereq.system_range('UID', path), prereq.system_range('GID', path)), (users, groups))
            os.unlink(path)
            self.assertEqual(prereq.system_range('UID', path), (101, 999))
        self.assertEqual(prereq.LOGIN_DEFINITIONS, '/etc/login.defs')

    def test_lookups_answer_none_instead_of_raising_for_what_is_not_there(self):
        def absent(key):
            raise KeyError(key)
        found = types.SimpleNamespace(gr_name='x', gr_gid=7)
        with patch.object(prereq.grp, 'getgrgid', absent), patch.object(prereq.grp, 'getgrnam', absent), \
                patch.object(prereq.pwd, 'getpwnam', absent):
            self.assertEqual((prereq.group_numbered(7), prereq.group('x'), prereq.account('x')), (None, None, None))
        with patch.object(prereq.grp, 'getgrgid', lambda gid: found), patch.object(prereq.grp, 'getgrnam', lambda name: found), \
                patch.object(prereq.pwd, 'getpwnam', lambda name: found):
            self.assertEqual((prereq.group_numbered(7), prereq.group('x'), prereq.account('x')), (found, found, found))
        with patch.object(prereq.os, 'getgrouplist', lambda name, gid: [gid, 42]):
            self.assertEqual(prereq.groups_of('x', 7), [7, 42])

    def test_the_tmpfiles_template_is_used_only_while_it_is_exactly_the_two_expected_lines(self):
        template = (prereq.TEMPLATES / 'studiolo-backup.tmpfiles.conf').read_text(encoding='utf-8')
        good = 'd /run/studiolo-backup 0750 maxsong studiolo-backup-read -\nd /var/lib/studiolo-backup-staging 0750 maxsong ' \
               'studiolo-backup-read -\n'
        first = 'd /run/studiolo-backup 0750 studiolo studiolo-backup-read -'
        self.assertIn(first + '\n', template)
        changes = {
            '权限放开': template.replace(first, first.replace('0750', '0755')),
            '换了属组': template.replace(first, first.replace('studiolo-backup-read', 'users')),
            '换了路径': template.replace(first, first.replace('/run/studiolo-backup', '/run/other')),
            '少一行': template.replace(first + '\n', ''),
            '多一行': template + 'd /var/lib/extra 0750 studiolo studiolo-backup-read -\n',
            '两行换了先后': template.replace(first + '\n', '') + first + '\n',
        }
        with tempfile.TemporaryDirectory() as scratch, patch.object(prereq, 'TEMPLATES', Path(scratch)):
            target = Path(scratch, 'studiolo-backup.tmpfiles.conf')
            for name, text in changes.items():
                with self.subTest(name):
                    target.write_text(text, encoding='utf-8')
                    with self.assertRaises(prereq.Refused) as refusal:
                        prereq.tmpfiles_text()
                    self.assertEqual(str(refusal.exception), '仓库里的 tmpfiles 片段不是预期的两行')
            # 只动注释不算数。
            target.write_text('# 另一句说明\n' + template, encoding='utf-8')
            self.assertEqual(prereq.tmpfiles_text(), good)
        self.assertEqual(prereq.tmpfiles_text(), good)


class InterruptionTests(unittest.TestCase):
    """终端断开（SIGHUP）、被要求结束（SIGTERM）和 Ctrl-C 一样：变成异常，让收尾照常跑。prereq.py 和 NAS 上的帮手各有一份。"""

    WATCHED = (signal.SIGHUP, signal.SIGTERM, signal.SIGINT)

    def setUp(self):
        for number in self.WATCHED:
            self.addCleanup(signal.signal, number, signal.getsignal(number))

    def test_each_signal_becomes_an_interruption_once_and_later_ones_are_ignored(self):
        for module in (prereq, nas):
            self.assertTrue(issubclass(module.Interrupted, KeyboardInterrupt))
            for number in self.WATCHED:
                with self.subTest(module=module.__name__, signal=number.name):
                    module.interruptible()
                    with self.assertRaises(module.Interrupted) as interruption:
                        os.kill(os.getpid(), number)
                        time.sleep(2)  # 等不满才对：信号一到就抛
                    self.assertEqual(str(interruption.exception), number.name)
                    # 收到第一个之后三个都不再理：收尾做到一半不会又被打断。
                    self.assertEqual([signal.getsignal(other) for other in self.WATCHED], [signal.SIG_IGN] * 3)
                    for other in self.WATCHED:
                        os.kill(os.getpid(), other)

    def test_a_file_being_placed_when_the_terminal_goes_away_leaves_nothing_behind(self):
        class Vanishing:
            def read(self, size):
                os.kill(os.getpid(), signal.SIGHUP)
                time.sleep(2)
                return b''
        with tempfile.TemporaryDirectory() as scratch:
            prereq.interruptible()
            with self.assertRaises(prereq.Interrupted):
                prereq.place(os.path.join(scratch, 'file'), 0o644, Vanishing(), None, digest(b''))
            self.assertEqual(os.listdir(scratch), [])

    def test_a_file_being_written_on_the_nas_when_ssh_goes_away_leaves_nothing_behind(self):
        real = os.fsync

        def vanishing(descriptor):
            os.kill(os.getpid(), signal.SIGHUP)
            time.sleep(2)
            real(descriptor)
        with tempfile.TemporaryDirectory() as scratch:
            status = os.lstat(scratch)
            nas.interruptible()
            with patch.object(nas.os, 'fsync', vanishing), self.assertRaises(nas.Interrupted):
                nas.write_new(Path(scratch, 'file'), b'half', (status.st_uid, status.st_gid), 0o640)
            self.assertEqual(os.listdir(scratch), [])


class ScriptEntryTests(unittest.TestCase):
    """三个脚本的入口：停下就是一句话加退出码 1，不是一屏回溯；没预料到的错不吞。"""

    def setUp(self):
        for number in InterruptionTests.WATCHED:
            self.addCleanup(signal.signal, number, signal.getsignal(number))

    def outcome(self, module, raised):
        def main(argv):
            if raised is not None:
                raise raised
        said = io.StringIO()
        with patch.object(module, 'main', main), contextlib.redirect_stderr(said):
            try:
                code = module.entry([])
            except KeyboardInterrupt:
                # 不让它一路传上去：那样整套用例会被当成「用户按了 Ctrl-C」直接中断，看不到是哪一条没过。
                self.fail('入口没有接住「被打断」')
        return code, said.getvalue()

    def test_prereq_turns_every_expected_stop_into_one_line_and_exit_code_one(self):
        failed = subprocess.CalledProcessError(3, ['sudo', '--', 'groupadd', '--system', 'x'])
        cases = (
            (prereq.Refused('现场不对'), '停下：现场不对\n'),
            (failed, '停下：命令失败（退出码 3）：sudo -- groupadd --system x\n'),
            (PermissionError(13, '不让写'), '停下：PermissionError：[Errno 13] 不让写\n'),
            (FileExistsError(17, '已经在', '/x'), "停下：FileExistsError：[Errno 17] 已经在: '/x'\n"),
            (KeyboardInterrupt(), '\n停下：被打断。正在放的文件不留半份，已经做完的步骤留着；重跑会从现状接着做。\n'),
            (prereq.Interrupted('SIGHUP'), '\n停下：被打断。正在放的文件不留半份，已经做完的步骤留着；重跑会从现状接着做。\n'),
        )
        for raised, told in cases:
            with self.subTest(type(raised).__name__):
                self.assertEqual(self.outcome(prereq, raised), (1, told))
        self.assertEqual(self.outcome(prereq, None), (0, ''))
        with self.assertRaises(ZeroDivisionError):
            self.outcome(prereq, ZeroDivisionError())
        # 入口先把三个信号接成「被打断」。
        self.assertNotIn(signal.getsignal(signal.SIGHUP), (signal.SIG_DFL, signal.SIG_IGN))

    def test_the_nas_helper_turns_every_expected_stop_into_one_line_and_exit_code_one(self):
        failed = subprocess.CalledProcessError(1, ['/usr/bin/systemctl', 'restart', nas.SERVICE])
        interrupted = ('nas_account=interrupted：被打断。写到一半的文件、演练用的临时单元已经收走；'
                       '单元要是已经换上，重跑 unit 会补上重新加载和重启\n')
        cases = (
            (ValueError('账号文件不是预期的样子'), 'nas_account=failed：账号文件不是预期的样子\n'),
            (OSError(28, '盘满了'), 'nas_account=failed：[Errno 28] 盘满了\n'),
            (failed, 'nas_account=failed：' + str(failed) + '\n'),
            (KeyboardInterrupt(), interrupted), (nas.Interrupted('SIGTERM'), interrupted),
        )
        for raised, told in cases:
            with self.subTest(type(raised).__name__):
                self.assertEqual(self.outcome(nas, raised), (1, told))
        self.assertEqual(self.outcome(nas, None), (0, ''))
        with self.assertRaises(ZeroDivisionError):
            self.outcome(nas, ZeroDivisionError())
        # 入口先把三个信号接成「被打断」（ssh 断了的时候，写到一半的文件才收得走）。
        self.assertNotIn(signal.getsignal(signal.SIGHUP), (signal.SIG_DFL, signal.SIG_IGN))

    def test_the_scripts_themselves_exit_with_one_and_one_line_when_they_refuse(self):
        """真的把脚本当程序跑一遍（参数不认识，在碰任何东西之前就被拒）：退出码 1，标准输出是空的，没有回溯。"""
        for name, argv, told in (('prereq.py', ['--force'], '停下：'), ('credentials.py', ['--force'], '停下：'),
                                 ('nas_account.py', ['bogus'], 'nas_account=failed：用法见文件开头；要经 sudo 跑\n')):
            with self.subTest(name):
                done = subprocess.run([sys.executable, '-I', '-B', str(HERE / name)] + argv, stdin=subprocess.DEVNULL,
                                      stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                said = done.stderr.decode()
                self.assertEqual((done.returncode, done.stdout), (1, b''))
                self.assertTrue(said.startswith(told), said)
                self.assertNotIn('Traceback', said)


if __name__ == '__main__':
    unittest.main()
