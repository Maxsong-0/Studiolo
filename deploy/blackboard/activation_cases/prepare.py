"""prepare 发行契约。"""
from .fixture import *


class PrepareTests(FilesBase):
    def test_prepare_swaps_web_and_carries_the_previous_program_sources(self):
        a, args = self.preparation()
        result = self.run_prepare(a, args)
        doc = self.check(a, result['manifest_sha256'])
        self.assertTrue(result['human_binary_reused'])
        self.assertEqual(result['web_files'], 2)
        # 程序来源照抄上一发行，不冒称新构建；网页来源来自本次参数。
        self.assertEqual(doc['bootstrap_source_commit'], '2' * 40)
        self.assertEqual(doc['human_program_source_commit'], '3' * 40)
        self.assertEqual(doc['web_source_commit'], args.web_source_commit)
        root = Path(a.RELEASE)
        self.assertEqual((root / 'bin/studiolo-local-server').read_bytes(), b'\x7fELFfictional human')
        self.assertEqual((root / 'bin/studiolo-auth-admin').read_bytes(), b'fictional admin')
        self.assertEqual((root / 'web/index.html').read_bytes(), b'brand new web')
        self.assertEqual((root / 'web/assets/brand-new.js').read_bytes(), b'brand new bundle')
        self.assertFalse((root / 'web/assets/stale.js').exists())
        with self.assertRaises(ValueError):
            self.run_prepare(a, args)

    def test_prepare_can_replace_the_human_program_and_records_its_new_source(self):
        a, args = self.preparation()
        program = b'\x7fELFnew human build'
        self.write(self.root / 'local-server', program)
        args.local_server = str(self.root / 'local-server')
        args.local_server_sha256 = c.sha(program)
        args.human_source_commit = '6' * 40
        result = self.run_prepare(a, args)
        self.assertFalse(result['human_binary_reused'])
        doc = self.check(a, result['manifest_sha256'])
        self.assertEqual(doc['human_program_source_commit'], '6' * 40)
        self.assertEqual(doc['bootstrap_source_commit'], '2' * 40)
        root = Path(a.RELEASE)
        self.assertEqual((root / 'bin/studiolo-local-server').read_bytes(), program)
        # 其余程序照旧逐字节沿用。
        self.assertEqual((root / 'bin/studiolo-auth-admin').read_bytes(), b'fictional admin')

    def test_prepare_rejects_partial_program_arguments_and_bad_program(self):
        a, args = self.preparation()
        program = b'\x7fELFnew human build'
        self.write(self.root / 'local-server', program)
        for patch_args in ({'local_server': str(self.root / 'local-server')},
                           {'local_server': str(self.root / 'local-server'),
                            'local_server_sha256': '0' * 64, 'human_source_commit': '6' * 40},
                           {'local_server': str(self.root / 'local-server'),
                            'local_server_sha256': c.sha(program), 'human_source_commit': 'z' * 40}):
            with self.subTest(args=sorted(patch_args)):
                trial = types.SimpleNamespace(**{**vars(args), **patch_args})
                with self.assertRaises(ValueError):
                    self.run_prepare(a, trial)
                self.assertFalse(Path(a.RELEASE).exists())
        (self.root / 'local-server').write_bytes(b'not ELF')
        trial = types.SimpleNamespace(**{**vars(args), 'local_server': str(self.root / 'local-server'),
                                         'local_server_sha256': c.sha(b'not ELF'),
                                         'human_source_commit': '6' * 40})
        with self.assertRaises(ValueError):
            self.run_prepare(a, trial)
        self.assertFalse(Path(a.RELEASE).exists())

    def test_prepare_rejects_bad_web_source_or_missing_index_before_creating_output(self):
        a, args = self.preparation()
        for bad in ('a0acdde', 'z' * 40, '5' * 39):
            prior = args.web_source_commit
            args.web_source_commit = bad
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                self.run_prepare(a, args)
            self.assertFalse(Path(a.RELEASE).exists())
            args.web_source_commit = prior
        (Path(args.web) / 'index.html').unlink()
        with self.assertRaises(ValueError):
            self.run_prepare(a, args)
        self.assertFalse(Path(a.RELEASE).exists())
        self.write(Path(args.web) / 'index.html', b'brand new web')
        # 现场单元与预期摘要不符时，同样在建目录之前就停。
        self.write(self.root / 'units' / 'studiolo-human.service.tmp', b'x')
        (self.root / 'units' / 'studiolo-human.service').write_bytes(b'drifted human')
        with self.assertRaises(ValueError):
            self.run_prepare(a, args)
        self.assertFalse(Path(a.RELEASE).exists())

