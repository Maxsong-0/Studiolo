#!/usr/bin/python3
"""真实虚构发行目录上的 Bridge 双程序打包、保留及拒绝契约。"""
from activation_cases.fixture import FilesBase, Path, c, json, unittest


class SyncReleaseTests(FilesBase):
    def inputs(self):
        a, args = self.preparation()
        args.human_source_commit = '6' * 40
        for option, data in (
                ('local_server', b'\x7fELFnew human'),
                ('maintain', b'\x7fELFnew maintenance'),
                ('sync_launcher', b'\x7fELFnew launcher')):
            path = self.root / (option + '.bin')
            self.write(path, data, 0o755)
            setattr(args, option, str(path))
            setattr(args, option + '_sha256', c.sha(data))
        return a, args

    def rewrite(self, a, doc):
        raw = (json.dumps(doc, sort_keys=True) + '\n').encode()
        (Path(a.RELEASE) / 'manifest.json').write_bytes(raw)
        return c.sha(raw)

    def test_prepare_delivers_both_executables_and_their_exact_source(self):
        a, args = self.inputs()
        receipt = self.run_prepare(a, args)
        doc = self.check(a, receipt['manifest_sha256'])
        self.assertEqual(doc['sync_program_source_commit'], args.human_source_commit)
        self.assertEqual(doc['human_program_source_commit'], args.human_source_commit)
        for option, name in (('maintain', 'studiolo-maintain'), ('sync_launcher', 'studiolo-sync-launcher')):
            file = Path(a.RELEASE) / 'bin' / name
            self.assertEqual(file.stat().st_mode & 0o777, 0o755)
            self.assertEqual(file.read_bytes(), Path(getattr(args, option)).read_bytes())
            self.assertEqual(doc['files']['bin/' + name]['sha256'], getattr(args, option + '_sha256'))
        self.assertFalse(receipt['services_changed'])
        self.assertFalse(receipt['human_binary_reused'])

    def test_partial_inputs_wrong_digest_or_nonelf_never_create_output(self):
        a, args = self.inputs()
        for field, bad in (('maintain', None), ('maintain_sha256', '0' * 64),
                           ('sync_launcher_sha256', None), ('local_server', None)):
            original = getattr(args, field)
            with self.subTest(field=field):
                setattr(args, field, bad)
                with self.assertRaises(ValueError):
                    self.run_prepare(a, args)
                self.assertFalse(Path(a.RELEASE).exists())
                setattr(args, field, original)
        Path(args.maintain).write_bytes(b'not ELF')
        args.maintain_sha256 = c.sha(b'not ELF')
        with self.assertRaises(ValueError):
            self.run_prepare(a, args)
        self.assertFalse(Path(a.RELEASE).exists())

    def test_candidate_rejects_partial_metadata_source_mismatch_and_nonelf(self):
        a, args = self.inputs()
        receipt = self.run_prepare(a, args)
        original = self.check(a, receipt['manifest_sha256'])
        for change in ('missing-source', 'one-tool', 'mismatched-source', 'nonelf', 'mode'):
            doc = json.loads(json.dumps(original))
            path = Path(a.RELEASE) / 'bin/studiolo-maintain'
            before = path.read_bytes()
            with self.subTest(change=change):
                if change == 'missing-source':
                    del doc['sync_program_source_commit']
                elif change == 'one-tool':
                    del doc['files']['bin/studiolo-maintain']
                elif change == 'mismatched-source':
                    doc['sync_program_source_commit'] = '7' * 40
                elif change == 'nonelf':
                    path.write_bytes(b'not ELF')
                    doc['files']['bin/studiolo-maintain'] = {'bytes': 7, 'sha256': c.sha(b'not ELF')}
                else:
                    path.chmod(0o600)
                with self.assertRaises(ValueError):
                    self.check(a, self.rewrite(a, doc))
                path.write_bytes(before)
                path.chmod(0o755)
        self.check(a, self.rewrite(a, original))

    def test_next_release_retains_tools_and_cannot_change_bytes_without_new_source(self):
        a, args = self.inputs()
        first = self.run_prepare(a, args)
        self.check(a, first['manifest_sha256'])
        a.OLD_RELEASE, a.R8_MANIFEST_SHA = a.RELEASE, first['manifest_sha256']
        a.RELEASE = str(self.root / 'next-release')
        for field in ('local_server', 'local_server_sha256', 'human_source_commit',
                      'maintain', 'maintain_sha256', 'sync_launcher', 'sync_launcher_sha256'):
            setattr(args, field, None)
        second = self.run_prepare(a, args)
        doc = self.check(a, second['manifest_sha256'])
        self.assertEqual(doc['sync_program_source_commit'], '6' * 40)
        path = Path(a.RELEASE) / 'bin/studiolo-maintain'
        changed = b'\x7fELFswapped maintenance'
        path.write_bytes(changed)
        doc['files']['bin/studiolo-maintain'] = {'bytes': len(changed), 'sha256': c.sha(changed)}
        with self.assertRaises(ValueError):
            self.check(a, self.rewrite(a, doc))
        # 来源只能随同批人侧程序的新来源一起推进，不能只重标旧工具。
        doc['sync_program_source_commit'] = '7' * 40
        with self.assertRaises(ValueError):
            self.check(a, self.rewrite(a, doc))
        doc['human_program_source_commit'] = '7' * 40
        self.check(a, self.rewrite(a, doc))

    def test_next_release_refuses_removing_both_tools_even_with_resigned_manifest(self):
        a, args = self.inputs()
        first = self.run_prepare(a, args)
        a.OLD_RELEASE, a.R8_MANIFEST_SHA = a.RELEASE, first['manifest_sha256']
        a.RELEASE = str(self.root / 'next-release')
        second = self.run_prepare(a, args)
        doc = self.check(a, second['manifest_sha256'])
        del doc['sync_program_source_commit']
        for name in ('studiolo-maintain', 'studiolo-sync-launcher'):
            del doc['files']['bin/' + name]
            (Path(a.RELEASE) / 'bin' / name).unlink()
        with self.assertRaises(ValueError):
            self.check(a, self.rewrite(a, doc))


if __name__ == '__main__':
    unittest.main()
