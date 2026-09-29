#!/usr/bin/python3
"""虚构文件与注入服务替身；不访问生产路径、网络、凭据或 systemd。"""
import importlib.util
import json
import os
from pathlib import Path
import signal
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent.parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


c = load('release_common', HERE.parent / 'human/release/release_common.py')
runtime = load('runtime_checks', HERE.parent / 'human/release/runtime_checks.py')
legacy = load('test_board_legacy', HERE.parent / 'human/release/activation_ops.py')
assembly = load('test_board_assembly', HERE / 'assembly.py')
activation = load('test_board_activate', HERE / 'activate.py')
for name in ('sync_tools', 'candidate', 'binding', 'snapshot'):
    load('board_' + name, HERE / (name + '.py'))
ops_module = load('test_board_ops', HERE / 'activation_ops.py')
prepare = load('test_board_prepare', HERE / 'prepare.py')
Operations = ops_module.Operations


class FilesBase(unittest.TestCase):
    def setUp(self):
        self.mask = os.umask(0o077)
        self.temp = tempfile.TemporaryDirectory(prefix='studiolo-board-fictional-')
        self.root = Path(self.temp.name).resolve()
        self.uid = os.geteuid()
        self.uid_patch = patch.object(c, 'UID', self.uid)
        self.uid_patch.start()
        # 临时沙盒的 /tmp 祖先不同于生产：只替换祖先遍历，保留文件/节点安全检查。
        def ancestors(path, uid):
            self.assertTrue(path.is_relative_to(self.root))
            for parent in path.parents:
                if parent == self.root.parent:
                    break
                s = parent.lstat()
                c.require(parent.is_dir() and not parent.is_symlink()
                          and s.st_uid == uid and s.st_mode & 0o7022 == 0)
        self.ancestor_patch = patch.object(c, 'ancestors', side_effect=ancestors)
        self.ancestor_patch.start()

    def tearDown(self):
        self.ancestor_patch.stop()
        self.uid_patch.stop()
        self.temp.cleanup()
        os.umask(self.mask)

    def write(self, path, raw, mode=0o600):
        path.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        c.new_file(path, raw, mode)

    def begin_record(self, directory):
        ops = Operations('0' * 64, assembly, c, Mock(), legacy)
        inspect = c.inspect
        # 不需要 root 跑单测：只将预期 root UID 映射为临时文件的真实主人。
        # 原 inspect 对真实 lstat 的类型、mode、链接和所有权检查全部保留。
        def inspect_fixture(path, uid, *args, **kwargs):
            self.assertEqual(uid, 0)
            return inspect(path, self.uid, *args, **kwargs)
        try:
            with patch.object(ops_module, 'RECORDS', directory), patch.object(c, 'UID', 1000), \
                    patch.object(c, 'inspect', side_effect=inspect_fixture), \
                    patch.object(ops_module.sys, 'platform', 'linux'), \
                    patch.object(os, 'getuid', return_value=0), patch.object(os, 'geteuid', return_value=0), \
                    patch.object(ops_module.pwd, 'getpwnam', return_value=types.SimpleNamespace(pw_uid=1000)):
                ops.begin()
        except BaseException:
            ops.close()
            raise
        return ops

    def candidate(self):
        """上一发行（r2 形状）与候选（r3）：只有 web/ 与启动器、人侧单元不同。"""
        old, new = self.root / 'old', self.root / 'new'
        def mode(name):
            return 0o755 if name in c.BINARIES | {'bin/studiolo-vault-admin'} else 0o600
        shared = {'bin/studiolo-local-server': b'\x7fELFfictional human',
                  'bin/studiolo-auth-admin': b'fictional admin',
                  'bin/studiolo-vault-admin': b'\x7fELFfictional bootstrap',
                  'studiolo-agent.service': b'old agent',
                  'example-vault/note.md': b'fictional note'}
        old_content = dict(shared, **{'launch-human.py': b'old launcher',
                                      'studiolo-human.service': b'old human',
                                      'web/index.html': b'stale web',
                                      'web/assets/stale.js': b'stale bundle'})
        for name, raw in old_content.items():
            self.write(old / name, raw, mode(name))
        files, _ = c.scan(old, self.uid)
        old_doc = dict(build='m5m6-board-20260920-r2', bootstrap_source_commit='2' * 40,
            human_program_source_commit='3' * 40, r8_manifest_sha256='0' * 64,
            assembly=assembly.ASSEMBLY,
            before_units={'studiolo-human': c.sha(b'older human'), 'studiolo-agent': c.sha(b'old agent')},
            files=files)
        raw = json.dumps(old_doc).encode()
        self.write(old / 'manifest.json', raw)
        a = types.SimpleNamespace(OLD_RELEASE=str(old), RELEASE=str(new), BUILD=assembly.BUILD,
            R8_MANIFEST_SHA=c.sha(raw), HUMAN_SHA=c.sha(b'old human'), AGENT_SHA=c.sha(b'old agent'),
            WEB_PREFIX=assembly.WEB_PREFIX, ASSEMBLY=assembly.ASSEMBLY)
        new_content = dict(shared, **{'launch-human.py': b'new launcher',
                                      'studiolo-human.service': b'new human',
                                      # agent 单元以现场为准：两次发行之间读取范围可能被改过。
                                      'studiolo-agent.service': b'current agent',
                                      'web/index.html': b'fresh web',
                                      'web/assets/fresh.js': b'fresh bundle'})
        for name, raw in new_content.items():
            self.write(new / name, raw, mode(name))
        files, _ = c.scan(new, self.uid)
        doc = dict(build=a.BUILD, bootstrap_source_commit='2' * 40, human_program_source_commit='3' * 40,
            web_source_commit='4' * 40, r8_manifest_sha256=a.R8_MANIFEST_SHA, assembly=assembly.ASSEMBLY,
            before_units={'studiolo-human': a.HUMAN_SHA, 'studiolo-agent': a.AGENT_SHA}, files=files)
        raw = json.dumps(doc).encode()
        self.write(new / 'manifest.json', raw)
        return a, c.sha(raw)

    def check(self, a, digest):
        verify = c.verify
        with patch.object(c, 'verify', side_effect=lambda root, sha: verify(root, sha, self.uid)):
            return ops_module.verify_candidate(a, c, digest)

    def preparation(self):
        a, _ = self.candidate()
        a.RELEASE = str(self.root / 'prepared')
        a.launcher = lambda text: text.replace('old', 'new')
        a.human_unit = lambda text: text.replace('old', 'new')
        a.agent_unit = lambda text: text
        for name, raw in (('studiolo-human', b'old human'), ('studiolo-agent', b'old agent')):
            self.write(self.root / 'units' / (name + '.service'), raw, 0o644)
        web = self.root / 'web-dist'
        self.write(web / 'index.html', b'brand new web')
        self.write(web / 'assets' / 'brand-new.js', b'brand new bundle')
        args = types.SimpleNamespace(web=str(web), web_source_commit='5' * 40,
                                     local_server=None, local_server_sha256=None,
                                     human_source_commit=None)
        return a, args

    def run_prepare(self, a, args):
        read, verify = c.read, c.verify
        def fixture_read(path, uid, mode=None):
            if path.parent == Path('/etc/systemd/system'):
                self.assertEqual(uid, 0)
                return read(self.root / 'units' / path.name, self.uid, mode)
            return read(path, uid, mode)
        with patch.object(c, 'read', side_effect=fixture_read), \
                patch.object(c, 'verify', side_effect=lambda root, sha: verify(root, sha, self.uid)):
            return prepare.prepare(args, a, c, ops_module)

    def broker(self):
        a = types.SimpleNamespace(VAULT=str(self.root / 'vault'), WRITER=str(self.root / 'writer'),
                                  BOARD=str(self.root / 'board'))
        vault, writer, board = [Path(p) for p in (a.VAULT, a.WRITER, a.BOARD)]
        paths = {'canonical': vault, 'data': writer, 'control': vault / '.workbench',
            'marker': vault / '.workbench/vault.json', 'staging': vault / '.workbench/staging',
            **{name: writer / name for name in ('history', 'idempotency', 'snapshots')}}
        for key, path in paths.items():
            if key != 'marker':
                path.mkdir(mode=0o700, parents=True, exist_ok=True)
        board.mkdir(mode=0o700)
        # 起点是已在跑的黑板：目录里有控制库（r2 起的现场样子）。
        self.write(board / 'control.sqlite3', b'fictional board')
        identifier = 'f0a2d4b6-5588-4411-9922-66778899aabb'
        self.write(paths['marker'], json.dumps({'format_version': 1, 'vault_uuid': identifier}).encode())
        doc = dict(format_version=1, pair_id=identifier, vault_uuid=identifier)
        for key, path in paths.items():
            s = path.stat()
            doc[key] = {'dev': s.st_dev, 'ino': s.st_ino}
        for path in (vault / '.workbench/broker-binding.json', writer / 'broker-binding.json'):
            self.write(path, json.dumps(doc).encode())
        self.write(writer / 'writer-state.json', b'fictional journal')
        return a

