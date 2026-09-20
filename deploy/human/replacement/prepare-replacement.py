#!/usr/bin/python3
"""仅由主人控制终端准备替换候选；不清库、不登记、不自行激活。"""
import getpass
import importlib.util
import json
import os
from pathlib import Path
import pwd
import secrets
import sqlite3
import sys

RELEASE = Path('/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r3')
CONFIG = Path('/home/maxsong/.config/studiolo-human/owner.json')
NEXT = Path('/etc/credstore.encrypted/studiolo-human-totp-v2')


def main():
    spec = importlib.util.spec_from_file_location('private_pairing', RELEASE/'pairing_support.py')
    pairing = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(pairing)
    pairing.protect()
    resume = sys.argv[1:] == ['--resume']
    pairing.require(resume or len(sys.argv) == 1)
    pairing.require(NEXT.exists() == resume)
    owner = pwd.getpwnam('maxsong')
    pairing.regular(CONFIG, owner.pw_uid, 0o600)
    config = json.loads(CONFIG.read_text())
    pairing.require(config['seed_ref'] == 'studiolo-human-totp-v1')
    database = Path(config['auth_directory'])/'auth.sqlite3'
    pairing.regular(database, owner.pw_uid, 0o600)
    with sqlite3.connect('file:' + str(database) + '?mode=ro', uri=True) as db:
        db.execute('PRAGMA trusted_schema=OFF')
        db.execute('PRAGMA query_only=ON')
        pairing.require(db.execute('PRAGMA user_version').fetchone() == (4,))
        pairing.require(db.execute('SELECT count(*) FROM totp_factor WHERE seed_ref=? AND enrollment_json IS NOT NULL',
                                   ('studiolo-human-totp-v1',)).fetchone() == (1,))
        for table in ('totp_pending', 'recovery_pending'):
            pairing.require(db.execute('SELECT count(*) FROM ' + table).fetchone() == (0,))
    pairing.CREDENTIAL, pairing.CREDENTIAL_ID = NEXT, 'totp-next'
    print('即将显示新因子临时配对画面。保留旧手机条目，完成网页替换后再移除。')
    pairing.require(getpass.getpass('只按回车开始；这里不输入验证码：') == '')
    seed = pairing.read_credential() if resume else secrets.token_hex(20)
    if not resume:
        pairing.create_credential(seed)
    def ready():
        pairing.require(getpass.getpass('\n添加手机条目并保存密钥后，只按回车返回；这里不输入验证码：') == '')
    pairing.pairing_screen(seed, ready, 'Studiolo 新因子（待切换）')
    print('新候选已安全保存；原因子尚未替换。回到Codex只回复“新因子准备好了”，不要贴终端内容。')


if __name__ == '__main__':
    try:
        main()
    except (Exception, KeyboardInterrupt):
        os.write(2, '候选准备未确认完成；保留状态，请让Codex核查。不要删除或覆盖文件。\n'.encode())
        raise SystemExit(1)
