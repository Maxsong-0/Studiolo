#!/usr/bin/python3
"""双工具输入与来源的独立契约；只读写本测试临时文件，不访问服务或生产。"""
import importlib.util
import os
from pathlib import Path
import stat
import sys
import tempfile
import types
import unittest
from unittest.mock import Mock, patch

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


common = load('sync_tools_common', HERE.parent / 'human/release/release_common.py')
tools = load('sync_tools_contract', HERE / 'sync_tools.py')


class Inputs(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='studiolo-sync-tools-fictional-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.uid = os.geteuid()

        # /tmp 是测试夹具边界：只替换祖先范围，文件类型/权限/链接与前后 pins 都走真实 read。
        def ancestors(path, uid):
            common.require(path.is_relative_to(self.root))
            for parent in path.parents:
                if parent == self.root.parent:
                    break
                value = parent.lstat()
                common.require(stat.S_ISDIR(value.st_mode) and value.st_uid == uid
                               and value.st_mode & 0o7022 == 0)

        ancestor_patch = patch.object(common, 'ancestors', side_effect=ancestors)
        ancestor_patch.start()
        self.addCleanup(ancestor_patch.stop)
        self.c = types.SimpleNamespace(UID=self.uid, read=common.read, absolute=common.absolute,
                                       require=common.require, sha=common.sha)
        self.raw = {'maintain': b'\x7fELFfictional maintain',
                    'sync_launcher': b'\x7fELFfictional launcher'}
        self.args = types.SimpleNamespace(local_server=str(self.root / 'human'),
            local_server_sha256='3' * 64, human_source_commit='4' * 40)
        for name, raw in self.raw.items():
            path = self.root / name
            common.new_file(path, raw, 0o755)
            setattr(self.args, name, str(path))
            setattr(self.args, name + '_sha256', common.sha(raw))

    def test_complete_pair_uses_safe_read_and_returns_exact_bytes(self):
        read = Mock(wraps=common.read)
        self.c.read = read
        result = tools.read_inputs(self.args, self.c)
        self.assertEqual(tools.TOOLS, frozenset({'bin/studiolo-maintain', 'bin/studiolo-sync-launcher'}))
        self.assertEqual(result, {'bin/studiolo-maintain': self.raw['maintain'],
                                 'bin/studiolo-sync-launcher': self.raw['sync_launcher']})
        self.assertEqual(read.call_count, 2)
        for call in read.call_args_list:
            self.assertEqual(call.args[1], self.uid)
            self.assertIsInstance(call.args[0], Path)

    def test_absent_optional_attributes_or_all_none_do_not_read(self):
        self.c.read = Mock(side_effect=AssertionError('must not read'))
        for args in (types.SimpleNamespace(), types.SimpleNamespace(maintain=None,
                maintain_sha256=None, sync_launcher=None, sync_launcher_sha256=None)):
            self.assertEqual(tools.read_inputs(args, self.c), {})
        self.c.read.assert_not_called()

    def test_each_missing_tool_or_human_field_is_rejected_before_reading(self):
        self.c.read = Mock(side_effect=AssertionError('must not read'))
        for field in ('maintain', 'maintain_sha256', 'sync_launcher', 'sync_launcher_sha256',
                      'local_server', 'local_server_sha256', 'human_source_commit'):
            for remove in (False, True):
                with self.subTest(field=field, remove=remove):
                    args = types.SimpleNamespace(**vars(self.args))
                    delattr(args, field) if remove else setattr(args, field, None)
                    with self.assertRaises(ValueError):
                        tools.read_inputs(args, self.c)
        self.c.read.assert_not_called()

    def test_wrong_digest_non_elf_and_invalid_digest_syntax_are_rejected(self):
        for name in self.raw:
            path = Path(getattr(self.args, name))
            with self.subTest(tool=name, failure='wrong digest'):
                args = types.SimpleNamespace(**vars(self.args))
                setattr(args, name + '_sha256', '0' * 64)
                with self.assertRaises(ValueError):
                    tools.read_inputs(args, self.c)
            for invalid in ('A' * 64, 'g' * 64, '1' * 63, '1' * 65, 123):
                args = types.SimpleNamespace(**vars(self.args))
                setattr(args, name + '_sha256', invalid)
                with self.subTest(tool=name, digest=invalid), self.assertRaises(ValueError):
                    tools.read_inputs(args, self.c)
            raw = b'fictional non-ELF executable'
            path.write_bytes(raw)
            args = types.SimpleNamespace(**vars(self.args))
            setattr(args, name + '_sha256', common.sha(raw))
            with self.subTest(tool=name, failure='non ELF'), self.assertRaises(ValueError):
                tools.read_inputs(args, self.c)
            path.write_bytes(self.raw[name])

    def test_invalid_source_or_path_is_value_error_without_input_disclosure(self):
        for source in ('A' * 40, 'g' * 40, '1' * 39, '1' * 41, '', 123):
            args = types.SimpleNamespace(**vars(self.args))
            args.human_source_commit = source
            with self.subTest(source=source), self.assertRaises(ValueError):
                tools.read_inputs(args, self.c)
        for path in ('relative', str(self.root / 'missing'), None, 17):
            args = types.SimpleNamespace(**vars(self.args))
            args.maintain = path
            with self.subTest(path=path), self.assertRaises(ValueError):
                tools.read_inputs(args, self.c)
        self.c.read = Mock(side_effect=OSError('fictional body must not escape'))
        with self.assertRaisesRegex(ValueError, '^sync tool inputs rejected$'):
            tools.read_inputs(self.args, self.c)

    def test_real_safe_read_rejects_links_multiple_links_and_writable_files(self):
        target = Path(self.args.maintain)
        link = self.root / 'linked'
        args = types.SimpleNamespace(**vars(self.args))
        args.maintain = str(link)
        link.symlink_to(target)
        with self.assertRaises(ValueError):
            tools.read_inputs(args, self.c)
        link.unlink()
        os.link(target, link)
        with self.assertRaises(ValueError):
            tools.read_inputs(self.args, self.c)
        link.unlink()
        target.chmod(0o777)
        with self.assertRaises(ValueError):
            tools.read_inputs(self.args, self.c)


def manifest(with_tools=False, source='1' * 40, human='1' * 40):
    # 本 helper 只判程序集合/来源；字节元数据的完整闭集由调用方独立核对。
    result = {'files': {'web/index.html': {}}, 'human_program_source_commit': human}
    if with_tools:
        result['files'].update({name: {} for name in tools.TOOLS})
        result['sync_program_source_commit'] = source
    return result


class Metadata(unittest.TestCase):
    def test_pair_and_empty_metadata_are_accepted(self):
        tools.validate_metadata(manifest(), common)
        tools.validate_metadata(manifest(True), common)

    def test_partial_pair_missing_source_and_extra_source_are_rejected(self):
        bad = []
        for name in tools.TOOLS:
            value = manifest(True)
            del value['files'][name]
            bad.append(value)
        value = manifest(True)
        del value['sync_program_source_commit']
        bad.append(value)
        for extra in (None, '1' * 40):
            value = manifest()
            value['sync_program_source_commit'] = extra
            bad.append(value)
        for value in bad:
            with self.subTest(manifest=value), self.assertRaises(ValueError):
                tools.validate_metadata(value, common)

    def test_malformed_files_or_source_are_rejected(self):
        for value in (None, [], {}, {'files': []}, {'files': None}):
            with self.subTest(manifest=value), self.assertRaises(ValueError):
                tools.validate_metadata(value, common)
        for source in (None, '', 1, 'A' * 40, 'g' * 40, 'a' * 39, 'a' * 41):
            with self.subTest(source=source), self.assertRaises(ValueError):
                tools.validate_metadata(manifest(True, source), common)

    def test_first_addition_and_source_update_allow_both_tools_only(self):
        for old, new in ((manifest(), manifest(True)),
                         (manifest(True), manifest(True, '2' * 40, '2' * 40))):
            self.assertEqual(tools.validate_transition(old, new, common), tools.TOOLS)

    def test_unchanged_source_or_absent_tools_allow_no_tool_byte_changes(self):
        self.assertEqual(tools.validate_transition(manifest(), manifest(), common), set())
        # 人侧独立更新而工具沿用：来源仍记旧提交，不允许更改工具字节。
        old, new = manifest(True), manifest(True, human='2' * 40)
        self.assertEqual(tools.validate_transition(old, new, common), set())
        self.assertEqual(tools.validate_transition(old, old, common), set())

    def test_addition_or_update_must_match_new_human_source(self):
        for old, new in ((manifest(), manifest(True, human='2' * 40)),
                         (manifest(True), manifest(True, '2' * 40, '3' * 40))):
            with self.subTest(old=old, new=new), self.assertRaises(ValueError):
                tools.validate_transition(old, new, common)
        new = manifest(True)
        del new['human_program_source_commit']
        with self.assertRaises(ValueError):
            tools.validate_transition(manifest(), new, common)

    def test_removal_and_malformed_old_or_new_metadata_are_refused(self):
        with self.assertRaises(ValueError):
            tools.validate_transition(manifest(True), manifest(), common)
        bad = manifest(True)
        bad['files'].pop('bin/studiolo-maintain')
        for old, new in ((bad, manifest(True)), (manifest(True), bad)):
            with self.subTest(old=old, new=new), self.assertRaises(ValueError):
                tools.validate_transition(old, new, common)


if __name__ == '__main__':
    unittest.main()
