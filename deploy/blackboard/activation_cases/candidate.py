"""candidate 发行契约。"""
from .fixture import *


class CandidateTests(FilesBase):
    def test_exact_package_swaps_web_and_reuses_everything_else(self):
        a, digest = self.candidate()
        doc = self.check(a, digest)
        self.assertEqual(doc['build'], assembly.BUILD)
        self.assertEqual(set(doc['before_units']), set(ops_module.NAMES))
        self.assertEqual(doc['bootstrap_source_commit'], '2' * 40)
        self.assertEqual(doc['human_program_source_commit'], '3' * 40)
        self.assertEqual(doc['web_source_commit'], '4' * 40)
        # 网页整块换掉：旧资源不再出现，新资源必须在。
        self.assertEqual({name for name in doc['files'] if name.startswith('web/')},
                         {'web/index.html', 'web/assets/fresh.js'})
        self.assertEqual((Path(a.RELEASE) / 'studiolo-agent.service').read_bytes(), b'current agent')
        for name in ('bin/studiolo-local-server', 'bin/studiolo-auth-admin',
                     'bin/studiolo-vault-admin', 'example-vault/note.md'):
            self.assertEqual((Path(a.OLD_RELEASE) / name).read_bytes(),
                             (Path(a.RELEASE) / name).read_bytes())
        self.assertNotEqual((Path(a.OLD_RELEASE) / 'web/index.html').read_bytes(),
                            (Path(a.RELEASE) / 'web/index.html').read_bytes())

    def test_tamper_extra_file_directory_missing_file_and_link_rejected(self):
        a, digest = self.candidate()
        root = Path(a.RELEASE)
        for name, kind in [('extra', 'file'), ('extra', 'directory'), ('linked', 'link')]:
            with self.subTest(kind=kind):
                path = root / name
                if kind == 'file':
                    self.write(path, b'unapproved')
                elif kind == 'directory':
                    path.mkdir(mode=0o700)
                else:
                    path.symlink_to(root / 'web/index.html')
                with self.assertRaises(ValueError):
                    self.check(a, digest)
                path.rmdir() if kind == 'directory' else path.unlink()
        target = root / 'web/index.html'
        target.unlink()
        with self.assertRaises(ValueError):
            self.check(a, digest)
        self.write(target, b'tampered')
        with self.assertRaises(ValueError):
            self.check(a, digest)

    def test_manifest_hash_and_closed_schema_rejected(self):
        a, digest = self.candidate()
        with self.assertRaises(ValueError):
            self.check(a, '0' * 64)
        path = Path(a.RELEASE) / 'manifest.json'
        doc = json.loads(path.read_bytes())
        doc['unrecognized'] = True
        raw = json.dumps(doc).encode()
        path.write_bytes(raw)
        with self.assertRaises(ValueError):
            self.check(a, c.sha(raw))

    def test_new_manifest_cannot_replace_binaries_or_example_vault(self):
        a, digest = self.candidate()
        root = Path(a.RELEASE)
        path = root / 'manifest.json'
        original = path.read_bytes()
        for name in ('bin/studiolo-auth-admin', 'bin/studiolo-local-server', 'example-vault/note.md'):
            with self.subTest(name=name):
                prior = (root / name).read_bytes()
                raw = b'changed file'
                (root / name).write_bytes(raw)
                doc = json.loads(original)
                doc['files'][name] = {'bytes': len(raw), 'sha256': c.sha(raw)}
                raw = json.dumps(doc).encode()
                path.write_bytes(raw)
                with self.assertRaises(ValueError):
                    self.check(a, c.sha(raw))
                (root / name).write_bytes(prior)
                path.write_bytes(original)
                self.check(a, digest)
        # 网页可以整块换，但首页必须在，来源提交也必须是规范的 40 位十六进制。
        (root / 'web/index.html').unlink()
        doc = json.loads(original)
        del doc['files']['web/index.html']
        raw = json.dumps(doc).encode()
        path.write_bytes(raw)
        with self.assertRaises(ValueError):
            self.check(a, c.sha(raw))
        self.write(root / 'web/index.html', b'fresh web')
        path.write_bytes(original)
        for bad in ('4' * 39, 'z' * 40, None):
            doc = json.loads(original)
            doc['web_source_commit'] = bad
            raw = json.dumps(doc).encode()
            path.write_bytes(raw)
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                self.check(a, c.sha(raw))
        path.write_bytes(original)

    def test_new_human_nonelf_invalid_source_or_digest_rejected(self):
        a, digest = self.candidate()
        root = Path(a.RELEASE)
        path = root / 'manifest.json'
        original = path.read_bytes()
        human = root / 'bin/studiolo-local-server'
        human.write_bytes(b'not ELF')
        doc = json.loads(original)
        doc['files']['bin/studiolo-local-server'] = {'bytes': 7, 'sha256': c.sha(b'not ELF')}
        raw = json.dumps(doc).encode()
        path.write_bytes(raw)
        with self.assertRaises(ValueError):
            self.check(a, c.sha(raw))
        path.write_bytes(original)
        human.write_bytes(b'\x7fELFtampered human')
        with self.assertRaises(ValueError):
            self.check(a, digest)
        human.write_bytes(b'\x7fELFnew human')
        for source in ('3' * 39, 'z' * 40, None):
            doc = json.loads(original)
            doc['human_program_source_commit'] = source
            raw = json.dumps(doc).encode()
            path.write_bytes(raw)
            with self.assertRaises(ValueError):
                self.check(a, c.sha(raw))

    def test_candidate_cannot_swap_the_program_without_declaring_a_new_source(self):
        a, digest = self.candidate()
        root = Path(a.RELEASE)
        path = root / 'manifest.json'
        doc = json.loads(path.read_bytes())
        swapped = b'\x7fELFsneaky human'
        (root / 'bin/studiolo-local-server').write_bytes(swapped)
        doc['files']['bin/studiolo-local-server'] = {'bytes': len(swapped), 'sha256': c.sha(swapped)}
        raw = json.dumps(doc).encode()
        path.write_bytes(raw)
        with self.assertRaises(ValueError):
            self.check(a, c.sha(raw))
        # 声明了新的来源提交才允许换。
        doc['human_program_source_commit'] = '7' * 40
        raw = json.dumps(doc).encode()
        path.write_bytes(raw)
        self.assertEqual(self.check(a, c.sha(raw))['human_program_source_commit'], '7' * 40)

