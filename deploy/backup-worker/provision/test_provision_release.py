"""prereq.py 换发行那一路的契约：现场是登记的上一版才换，上一版留在原地；别的样子照旧停下。

前一半对着 test_provision_prereq.py 的内存现场跑整套流程；后一半把真的 replace、superseded、occupied 放到临时目录里跑。
由同目录的 test_provision.py 并进去一起跑（合并门禁只跑那一份），也可以单独跑。
"""
import hashlib
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

HERE = Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


# 内存现场和它打补丁的那一份 prereq 要是同一份：从那边取，不另外加载。用例类只经 site 取，不在这里起名字
# （起了名字，并进 test_provision.py 时会被当成这一份的用例再跑一遍）。
site = load('test_provision_prereq')
prereq, digest, label = site.prereq, site.digest, site.label
WORKER, READERS = prereq.WORKER, prereq.READERS
# 虚构的上一版：三样程序的内容和这一版都不一样。
OLD = {name: ('previous ' + name).encode() for name, _ in site.PACKAGES}
BEFORE = {'commit': 'abc1234', 'release': 'fictional-r1', 'packages': tuple((name, digest(OLD[name])) for name in OLD)}
OLD_WORKER = OLD['studiolo-backup-worker']
PROGRAM, CONFIG = 'worker 程序', '私有备份配置 ' + prereq.BACKUP_CONFIG + '（不用 sudo，0600）'
QUERIES = site.GATE[2:]
STILL_ON = ' 还启用着或在跑：换 worker 程序之前先停掉（sudo systemctl disable --now 加这两个单元的名字）'


def kept(path):
    return prereq.retired(path)


class ReleaseChangeTests(unittest.TestCase):
    def provision(self, world, apply=True):
        self.lines = []
        return prereq.Provision(apply, run=world.run, out=self.lines.append)

    def previous_site(self):
        """照上一版置备好的现场：worker 程序和私有备份配置是上一版的，其余和这一版一样。"""
        world = site.fresh()
        with world.installed():
            self.assertEqual(self.provision(world).everything(), site.STEPS)
            numbers = (world.accounts[WORKER].pw_uid, world.groups[READERS].gr_gid)
            self.old_config = prereq.encode(prereq.backup_document(*numbers, BEFORE))
            self.new_config = prereq.encode(prereq.backup_document(*numbers))
        world.nodes[prereq.WORKER_BINARY].raw = OLD_WORKER
        world.nodes[prereq.BACKUP_CONFIG].raw = self.old_config
        world.ran.clear()
        return world

    @staticmethod
    def changing(world, previous=BEFORE):
        stack = world.installed()
        return _both(stack, patch.object(prereq, 'PREVIOUS', previous))

    def test_the_kept_name_follows_this_release_and_the_previous_document_is_reproduced_exactly(self):
        self.assertEqual(prereq.RETIRED_SUFFIX, '.before-' + prereq.HUMAN_RELEASE.rsplit('-', 1)[1])
        self.assertEqual(kept('/x/y'), '/x/y' + prereq.RETIRED_SUFFIX)
        now, before = prereq.backup_document(7, 8), prereq.backup_document(7, 8, BEFORE)
        producer = before['backup_producer']
        self.assertEqual([entry['version'] for entry in producer['application_packages']], ['abc1234'] * 3)
        self.assertEqual([entry['digest'] for entry in producer['application_packages']],
                         [digest(raw) for raw in OLD.values()])
        self.assertEqual([entry['revision'] for entry in producer['deployment']], ['fictional-r1', 'abc1234'])
        # 别的字段一个不差：上一版那一份只有这三处和这一版不同。
        for document in (now, before):
            del document['backup_producer']['application_packages'], document['backup_producer']['deployment']
        self.assertEqual(now, before)

    def test_the_registered_previous_release_is_swapped_and_kept_and_a_second_run_changes_nothing(self):
        world = self.previous_site()
        before = world.snapshot()
        with self.changing(world):
            # 预演：只读地问两个单元停没停，说清要换哪两样，什么都不动。
            self.assertEqual(self.provision(world, apply=False).everything(), 2)
            self.assertEqual((world.snapshot(), [label(argv) for argv in world.ran]), (before, QUERIES))
            for title, path in ((PROGRAM, prereq.WORKER_BINARY), (CONFIG, prereq.BACKUP_CONFIG)):
                self.assertIn('[要换] ' + title + '（现场是上一版；换上这一版，上一版留在 ' + kept(path) + '）', self.lines)
            self.assertFalse([line for line in self.lines if line.startswith(('[要做] ', '[现在换] ', '[现在做] '))])
            world.ran.clear()
            # 只读核对那一路（credentials.py 用）：现场还是上一版，不算置备好。
            with self.assertRaises(prereq.Refused) as refusal:
                world.settled()
            self.assertEqual(str(refusal.exception), PROGRAM + '：现场还是上一版。先跑完 prereq.py --apply')
            self.assertEqual(world.snapshot(), before)
            world.ran.clear()
            # 照做：两样都换成这一版，上一版原样留在旁边；先问单元停没停，再换程序。
            self.assertEqual(self.provision(world).everything(), 2)
            nodes = world.snapshot()[0]
            self.assertEqual(nodes[prereq.WORKER_BINARY], ('file', 0, 0, 0o755, site.FICTION['worker'], 1))
            self.assertEqual(nodes[kept(prereq.WORKER_BINARY)], ('file', 0, 0, 0o755, OLD_WORKER, 1))
            owned = ('file', prereq.OWNER_UID, prereq.OWNER_UID, 0o600)
            self.assertEqual(nodes[prereq.BACKUP_CONFIG], owned + (self.new_config, 1))
            self.assertEqual(nodes[kept(prereq.BACKUP_CONFIG)], owned + (self.old_config, 1))
            self.assertEqual(sorted(set(nodes) - set(before[0])), sorted(map(kept, (prereq.WORKER_BINARY, prereq.BACKUP_CONFIG))))
            self.assertEqual({path: nodes[path] for path in before[0]
                              if path not in (prereq.WORKER_BINARY, prereq.BACKUP_CONFIG)},
                             {path: node for path, node in before[0].items()
                              if path not in (prereq.WORKER_BINARY, prereq.BACKUP_CONFIG)})
            self.assertEqual([label(argv) for argv in world.ran], QUERIES + ['replace ' + prereq.WORKER_BINARY] + site.GATE)
            self.assertEqual(len([line for line in self.lines if line.startswith('[现在换] ')]), 2)
            # 第二遍：已经是这一版，一步都不用做；留下的上一版不碍事，也不再动。
            swapped = world.snapshot()
            world.ran.clear()
            self.assertEqual(self.provision(world).everything(), 0)
            self.assertEqual((world.snapshot(), [label(argv) for argv in world.ran]), (swapped, site.GATE))
            self.assertEqual(world.settled(), site.STEPS)

    def test_a_worker_program_identical_to_the_previous_release_is_left_alone_and_only_the_config_is_swapped(self):
        # 两版的 worker 程序从同样的源码编出来、一字不差（r21→r22 就是这样）：现场那一份已经是这一版，
        # 没有可换的。不换、不留上一版的名字、不问单元停没停；只读核对那一路也认它是这一版。
        same = dict(BEFORE, packages=BEFORE['packages'][:2] + (('studiolo-backup-worker', site.PACKAGES[2][1]),))
        world = self.previous_site()
        world.nodes[prereq.WORKER_BINARY].raw = site.FICTION['worker']
        with self.changing(world, same):
            numbers = (world.accounts[WORKER].pw_uid, world.groups[READERS].gr_gid)
            old_config = prereq.encode(prereq.backup_document(*numbers, same))
            world.nodes[prereq.BACKUP_CONFIG].raw = old_config
            before = world.snapshot()
            self.assertEqual(self.provision(world, apply=False).everything(), 1)
            self.assertEqual((world.snapshot(), world.ran), (before, []))
            self.assertIn('[已有] ' + PROGRAM, self.lines)
            self.assertIn('[要换] ' + CONFIG + '（现场是上一版；换上这一版，上一版留在 '
                          + kept(prereq.BACKUP_CONFIG) + '）', self.lines)
            with self.assertRaises(prereq.Refused) as refusal:
                world.settled()
            self.assertEqual(str(refusal.exception), CONFIG + '：现场还是上一版。先跑完 prereq.py --apply')
            world.ran.clear()
            self.assertEqual(self.provision(world).everything(), 1)
            nodes = world.snapshot()[0]
            self.assertEqual(nodes[prereq.WORKER_BINARY], ('file', 0, 0, 0o755, site.FICTION['worker'], 1))
            owned = ('file', prereq.OWNER_UID, prereq.OWNER_UID, 0o600)
            self.assertEqual(nodes[prereq.BACKUP_CONFIG], owned + (self.new_config, 1))
            self.assertEqual(nodes[kept(prereq.BACKUP_CONFIG)], owned + (old_config, 1))
            self.assertEqual(sorted(set(nodes) - set(before[0])), [kept(prereq.BACKUP_CONFIG)])
            self.assertEqual([label(argv) for argv in world.ran], site.GATE)
            # 第二遍一步都不用做，只读核对认全部。
            swapped = world.snapshot()
            world.ran.clear()
            self.assertEqual(self.provision(world).everything(), 0)
            self.assertEqual((world.snapshot(), [label(argv) for argv in world.ran]), (swapped, site.GATE))
            self.assertEqual(world.settled(), site.STEPS)

    def test_nothing_is_swapped_unless_the_site_is_exactly_the_registered_previous_release(self):
        cases = [
            ('别的内容', prereq.WORKER_BINARY, {'raw': b'neither release'}, BEFORE, PROGRAM),
            ('上一版的内容、权限不对', prereq.WORKER_BINARY, {'mode': 0o555}, BEFORE, PROGRAM),
            ('上一版的内容、属主不对', prereq.WORKER_BINARY, {'uid': 12}, BEFORE, PROGRAM),
            ('上一版的内容、多一个不相干的名字', prereq.WORKER_BINARY, {'links': 2}, BEFORE, PROGRAM),
            ('脚本里没有登记上一版', prereq.WORKER_BINARY, {}, None, PROGRAM),
            ('配置是别的内容', prereq.BACKUP_CONFIG, {'raw': b'{}\n'}, BEFORE, CONFIG),
            ('配置是上一版的内容、权限不对', prereq.BACKUP_CONFIG, {'mode': 0o644}, BEFORE, CONFIG),
        ]
        for name, path, change, previous, title in cases:
            with self.subTest(name):
                world = self.previous_site()
                if path == prereq.BACKUP_CONFIG:
                    # 只看配置这一样：程序先放成这一版的，免得停在程序上。
                    world.nodes[prereq.WORKER_BINARY].raw = site.FICTION['worker']
                vars(world.nodes[path]).update(change)
                before = world.snapshot()
                with self.changing(world, previous), self.assertRaises(prereq.Refused) as refusal:
                    self.provision(world).everything()
                self.assertEqual(str(refusal.exception), title + site.WRONG)
                # 一样没动；要 root 的命令一条都没跑。
                self.assertEqual(world.snapshot(), before)
                self.assertFalse([argv for argv in world.ran if argv[:2] == ['sudo', '--']])

    def test_the_worker_program_is_not_swapped_while_the_worker_or_the_timer_is_enabled_or_running(self):
        for unit in site.GATED:
            for query, answer in (('is-enabled', 'enabled'), ('is-active', 'active'), ('is-active', 'failed')):
                for apply in (False, True):
                    with self.subTest(unit=unit, query=query, answer=answer, apply=apply):
                        world = self.previous_site()
                        world.units[unit][query] = answer
                        before = world.snapshot()
                        with self.changing(world), self.assertRaises(prereq.Refused) as refusal:
                            self.provision(world, apply).everything()
                        self.assertEqual(str(refusal.exception), unit + STILL_ON)
                        self.assertEqual(world.snapshot(), before)
                        self.assertFalse([argv for argv in world.ran if argv[:2] == ['sudo', '--']])

    def test_a_kept_name_taken_by_something_else_stops_the_swap_and_a_half_done_swap_is_finished(self):
        for path, title in ((prereq.WORKER_BINARY, PROGRAM), (prereq.BACKUP_CONFIG, CONFIG)):
            with self.subTest(taken=path):
                world = self.previous_site()
                if path == prereq.BACKUP_CONFIG:
                    world.nodes[prereq.WORKER_BINARY].raw = site.FICTION['worker']
                world.file(kept(path), b'somebody else put this here', 0, 0, 0o644)
                before = world.snapshot()
                with self.changing(world), self.assertRaises(prereq.Refused) as refusal:
                    self.provision(world).everything()
                self.assertEqual(str(refusal.exception), kept(path) + prereq.OCCUPIED)
                self.assertEqual(world.snapshot(), before)
                self.assertFalse([argv for argv in world.ran if argv[:2] == ['sudo', '--']])
        # 上一次起好了留下用的名字、还没换上就被打断：两个名字是同一份。重跑接着换完。
        world = self.previous_site()
        for path in (prereq.WORKER_BINARY, prereq.BACKUP_CONFIG):
            world.nodes[path].links = 2
            world.nodes[kept(path)] = world.nodes[path]
        with self.changing(world):
            self.assertEqual(self.provision(world).everything(), 2)
            nodes = world.snapshot()[0]
            self.assertEqual((nodes[prereq.WORKER_BINARY][4:], nodes[kept(prereq.WORKER_BINARY)][4:]),
                             ((site.FICTION['worker'], 1), (OLD_WORKER, 1)))
            self.assertEqual((nodes[prereq.BACKUP_CONFIG][4:], nodes[kept(prereq.BACKUP_CONFIG)][4:]),
                             ((self.new_config, 1), (self.old_config, 1)))

    def test_a_source_that_changes_after_the_check_or_a_leftover_temporary_name_leaves_the_previous_release_in_place(self):
        world = self.previous_site()
        world.swapped[prereq.WORKER_SOURCE] = b'swapped after the check'
        before = world.snapshot()
        with self.changing(world), self.assertRaises(prereq.subprocess.CalledProcessError):
            self.provision(world).everything()
        self.assertEqual(world.snapshot(), before)
        world = self.previous_site()
        world.file(prereq.placing(prereq.WORKER_BINARY), b'half', 0, 0, 0o600)
        before = world.snapshot()
        with self.changing(world), self.assertRaises(prereq.Refused) as refusal:
            self.provision(world).everything()
        self.assertEqual(str(refusal.exception), prereq.placing(prereq.WORKER_BINARY) + prereq.LEFTOVER)
        self.assertEqual(world.snapshot(), before)

    def test_the_replace_entry_only_serves_root_and_only_the_registered_swap_of_the_worker_program(self):
        old, new = BEFORE['packages'][2][1], site.PACKAGES[2][1]
        piped = site.types.SimpleNamespace(platform='linux', stdin=site.types.SimpleNamespace(
            isatty=lambda: False, buffer=io.BytesIO(site.FICTION['worker'])))
        refused = '--replace 只由本脚本自己经 sudo 调用，只换 worker 程序，只从登记的上一版换到这一版'
        cases = [
            ('不是 root', 1000, BEFORE, [prereq.WORKER_BINARY, old, new], piped),
            ('别的目标', 0, BEFORE, [prereq.RESTIC, old, new], piped),
            ('不是登记的上一版摘要', 0, BEFORE, [prereq.WORKER_BINARY, digest(b'x'), new], piped),
            ('不是这一版的摘要', 0, BEFORE, [prereq.WORKER_BINARY, old, digest(b'x')], piped),
            ('没有登记上一版', 0, None, [prereq.WORKER_BINARY, old, new], piped),
            ('少一个参数', 0, BEFORE, [prereq.WORKER_BINARY, old], piped),
            ('标准输入是终端', 0, BEFORE, [prereq.WORKER_BINARY, old, new], site.types.SimpleNamespace(
                platform='linux', stdin=site.types.SimpleNamespace(isatty=lambda: True, buffer=io.BytesIO(b'')))),
        ]
        for name, euid, previous, arguments, stdin in cases:
            with self.subTest(name):
                world = self.previous_site()
                before = world.snapshot()
                with self.changing(world, previous), patch.object(prereq.os, 'geteuid', return_value=euid), \
                        patch.object(prereq, 'sys', stdin), self.assertRaises(prereq.Refused) as refusal:
                    prereq.main(['--replace'] + arguments)
                self.assertEqual((str(refusal.exception), world.snapshot()), (refused, before))


def _both(*managers):
    """把几个上下文管理器合成一个（with 里好写）。"""
    import contextlib

    @contextlib.contextmanager
    def together():
        with contextlib.ExitStack() as stack:
            for manager in managers:
                stack.enter_context(manager)
            yield
    return together()


class RealReplaceTests(unittest.TestCase):
    """真的 replace、superseded、occupied，对着临时目录里的真文件。"""
    OLD, NEW = b'the previous release\n', b'this release\n'

    def setUp(self):
        scratch = tempfile.TemporaryDirectory()
        self.addCleanup(scratch.cleanup)
        self.target = os.path.join(scratch.name, 'program')
        self.me = (os.geteuid(), os.getegid())
        with open(self.target, 'wb') as out:
            out.write(self.OLD)
        os.chmod(self.target, 0o755)

    def listing(self):
        directory = os.path.dirname(self.target)
        return {name: (Path(directory, name).read_bytes(), os.lstat(os.path.join(directory, name)).st_nlink)
                for name in sorted(os.listdir(directory))}

    def swap(self, raw=None, previous=None, wanted=None, mode=0o755):
        raw = self.NEW if raw is None else raw
        prereq.replace(self.target, mode, io.BytesIO(raw), None, previous or digest(self.OLD), wanted or digest(self.NEW))

    def test_the_previous_release_is_kept_under_its_own_name_and_the_new_one_takes_the_real_name(self):
        self.assertTrue(prereq.superseded(self.target, *self.me, 0o755, digest(self.OLD)))
        self.assertFalse(prereq.occupied(self.target))
        self.swap()
        self.assertEqual(self.listing(), {'program': (self.NEW, 1), 'program' + prereq.RETIRED_SUFFIX: (self.OLD, 1)})
        self.assertEqual(os.lstat(self.target).st_mode & 0o7777, 0o755)
        self.assertEqual(os.lstat(kept(self.target)).st_mode & 0o7777, 0o755)
        # 换完之后现场是这一版：不再认成上一版，再换一次被拒，什么都不动。
        self.assertFalse(prereq.superseded(self.target, *self.me, 0o755, digest(self.OLD)))
        done = self.listing()
        with self.assertRaises(prereq.Refused) as refusal:
            self.swap()
        self.assertEqual(str(refusal.exception), self.target + ' 不是登记的上一版。什么都没换，先由人看过')
        self.assertEqual(self.listing(), done)

    def test_nothing_changes_when_the_site_the_content_or_the_kept_name_is_not_what_was_checked(self):
        before = self.listing()
        with self.assertRaises(prereq.Refused):
            self.swap(previous=digest(b'something else'))
        self.assertEqual(self.listing(), before)
        # 要放的内容不是核过的那一份：不换，临时名不留。
        with self.assertRaises(prereq.Refused) as refusal:
            self.swap(raw=b'swapped after the check')
        self.assertIn('要放的内容和核过的那一份不一样', str(refusal.exception))
        self.assertEqual(self.listing(), before)
        # 留下用的名字被别的东西占着：不覆盖它，也不换。
        with open(kept(self.target), 'wb') as out:
            out.write(b'somebody else')
        taken = self.listing()
        self.assertTrue(prereq.occupied(self.target))
        with self.assertRaises(prereq.Refused) as refusal:
            self.swap()
        self.assertEqual(str(refusal.exception), kept(self.target) + prereq.OCCUPIED)
        self.assertEqual(self.listing(), taken)
        os.unlink(kept(self.target))
        # 上一次留下的临时名还在：停下，那一份不动。
        with open(prereq.placing(self.target), 'wb') as out:
            out.write(b'half')
        stale = self.listing()
        with self.assertRaises(prereq.Refused) as refusal:
            self.swap()
        self.assertEqual(str(refusal.exception), prereq.placing(self.target) + prereq.LEFTOVER)
        self.assertEqual(self.listing(), stale)

    def test_a_swap_cut_off_after_the_previous_release_got_its_kept_name_is_finished_by_the_next_run(self):
        os.link(self.target, kept(self.target))
        self.assertTrue(prereq.superseded(self.target, *self.me, 0o755, digest(self.OLD)))
        self.assertFalse(prereq.occupied(self.target))
        self.swap()
        self.assertEqual(self.listing(), {'program': (self.NEW, 1), 'program' + prereq.RETIRED_SUFFIX: (self.OLD, 1)})

    def test_only_a_plain_singly_named_file_with_the_right_owner_mode_and_content_counts_as_the_previous_release(self):
        old = digest(self.OLD)
        self.assertFalse(prereq.superseded(self.target, *self.me, 0o700, old))
        self.assertFalse(prereq.superseded(self.target, self.me[0] + 1, self.me[1], 0o755, old))
        self.assertFalse(prereq.superseded(self.target, *self.me, 0o755, digest(self.NEW)))
        self.assertFalse(prereq.superseded(self.target + '.missing', *self.me, 0o755, old))
        # 多出来的名字不是留下用的那个：不认。
        os.link(self.target, self.target + '.alias')
        self.assertFalse(prereq.superseded(self.target, *self.me, 0o755, old))
        os.unlink(self.target + '.alias')
        # 链接不跟：指向上一版内容的链接不算。
        os.rename(self.target, self.target + '.real')
        os.symlink(self.target + '.real', self.target)
        self.assertFalse(prereq.superseded(self.target, *self.me, 0o755, old))

    def test_an_interruption_while_the_new_copy_is_being_written_leaves_the_previous_release_alone(self):
        class Cut(io.BytesIO):
            def read(self, size=-1):
                raise prereq.Interrupted('SIGHUP')
        before = self.listing()
        with self.assertRaises(prereq.Interrupted):
            prereq.replace(self.target, 0o755, Cut(self.NEW), None, digest(self.OLD), digest(self.NEW))
        self.assertEqual(self.listing(), before)


if __name__ == '__main__':
    unittest.main()
