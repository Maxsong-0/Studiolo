#!/usr/bin/python3
"""仅合成秘密：验证临时画面退出与密文发布失败边界。"""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch
import sys

spec = importlib.util.spec_from_file_location('pairing', sys.argv[1])
pairing = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pairing)
del sys.argv[1]


class PairingTests(unittest.TestCase):
    def screen(self, fail):
        writes = []
        seen = []
        def finish():
            seen.append(True)
            if fail:
                raise KeyboardInterrupt()
        with patch.object(pairing.os, 'write', side_effect=lambda fd, raw: writes.append(raw)), contextlib.redirect_stdout(io.StringIO()):
            if fail:
                with self.assertRaises(KeyboardInterrupt):
                    pairing.pairing_screen('00'*20, finish)
            else:
                pairing.pairing_screen('00'*20, finish)
        self.assertEqual(seen, [True])
        self.assertEqual(writes[0], b'\x1b[?1049h\x1b[2J\x1b[H')
        self.assertEqual(writes[-1], b'\x1b[2J\x1b[H\x1b[?1049l')

    def test_screen_closes_on_success(self): self.screen(False)
    def test_screen_closes_on_interrupt(self): self.screen(True)

    def encryption(self, root, exists=False, fail_sync=False):
        pairing.CREDENTIAL=Path(root)/'candidate.cred'
        pairing.CREDENTIAL_ID='totp-next'
        if exists:
            pairing.CREDENTIAL.write_bytes(b'existing-fictional-cipher')
        real_lstat=Path.lstat
        def metadata(path):
            value=real_lstat(path)
            return types.SimpleNamespace(st_mode=value.st_mode,st_uid=0)
        captured=[]
        def encrypt(args, **kwargs):
            captured.append(args)
            return types.SimpleNamespace(returncode=0,stdout=b'fictional-cipher')
        with patch.object(Path,'lstat',metadata),patch.object(pairing.subprocess,'run',side_effect=encrypt):
            if exists:
                with self.assertRaises(FileExistsError): pairing.create_credential('00'*20)
                self.assertEqual(pairing.CREDENTIAL.read_bytes(),b'existing-fictional-cipher')
            elif fail_sync:
                with patch.object(pairing.os,'fsync',side_effect=OSError('fixture')):
                    with self.assertRaises(OSError): pairing.create_credential('00'*20)
                self.assertFalse(pairing.CREDENTIAL.exists())
            else:
                pairing.create_credential('00'*20)
                self.assertEqual(pairing.CREDENTIAL.read_bytes(),b'fictional-cipher')
                self.assertEqual(pairing.CREDENTIAL.stat().st_nlink,1)
                self.assertEqual(pairing.CREDENTIAL.stat().st_mode&0o777,0o600)
        self.assertIn('--name=totp-next',captured[0])
        self.assertEqual(list(Path(root).glob('.studiolo-human-totp-*')),[])

    def test_atomic_cipher_publish(self):
        with tempfile.TemporaryDirectory() as root:self.encryption(root)
    def test_existing_cipher_not_overwritten(self):
        with tempfile.TemporaryDirectory() as root:self.encryption(root,exists=True)
    def test_failed_cipher_sync_not_published(self):
        with tempfile.TemporaryDirectory() as root:self.encryption(root,fail_sync=True)


unittest.main()
