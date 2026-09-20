#!/usr/bin/python3
"""仅在主人私有终端重新交付仍生效的旧因子；不生成秘密或OTP、不改认证状态。"""
import base64
import getpass
import importlib.util
import json
import os
from pathlib import Path
import pwd
import sqlite3
import subprocess
import sys
import warnings

RELEASE = Path('/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r6-restore-old')
CONFIG = Path('/home/maxsong/.config/studiolo-human/owner.json')
CURRENT = Path('/var/lib/studiolo-human-quarantine/t092-exposed-enrollment-yaghjdqh/retired-totp.cred')
STAGE = 'startup'


def step(name):
    global STAGE
    STAGE = name


def write_all(fd, value):
    while value:
        count = os.write(fd, value)
        if count <= 0:
            raise OSError('terminal_write')
        value = value[count:]


def enter_only(prompt):
    # 非空输入不回显、不记录；重新提示，不把误输入混成系统失败。
    with warnings.catch_warnings():
        warnings.simplefilter('error', getpass.GetPassWarning)
        while getpass.getpass(prompt) != '':
            write_all(2, '这里不用输入任何内容，请只按回车；或按 Ctrl+C 退出。\n'.encode())


def validate_state(pairing):
    step('state')
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


def read_current(pairing):
    step('credential-file')
    pairing.regular(CURRENT, 0, 0o600)
    step('decrypt')
    result = subprocess.run(
        ['/usr/bin/systemd-creds', 'decrypt', '--name=totp-seed', '--newline=no', str(CURRENT), '-'],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        env={'PATH': '/usr/bin:/bin', 'LANG': 'C.UTF-8'}, check=False,
    )
    pairing.require(result.returncode == 0)
    step('credential-shape')
    pairing.require(len(result.stdout) == 40 and all(c in b'0123456789abcdef' for c in result.stdout))
    return result.stdout.decode('ascii')


def show_current(seed):
    step('format')
    encoded = base64.b32encode(bytes.fromhex(seed)).decode('ascii').rstrip('=')
    screen = ('手机验证器：手动添加；名称 Studiolo 旧因子（仅用于替换）；基于时间；SHA1；6位；30秒。\n'
              '这是已暴露、尚待替换的旧因子，仅用于完成原生替换；不要覆盖手机里的新因子条目。不要截图或复制到聊天。\n'
              '配对密钥：' + encoded + '\n').encode('utf-8')
    step('screen-enter')
    try:
        write_all(1, b'\x1b[?1049h\x1b[2J\x1b[H')
        step('screen-write')
        write_all(1, screen)
        step('screen-confirm')
        enter_only('\n旧条目已加回手机后，只按回车返回；这里不输入验证码：')
    finally:
        try:
            write_all(1, b'\x1b[2J\x1b[H\x1b[?1049l')
        except (Exception, KeyboardInterrupt):
            step('screen-exit')
            raise


def main():
    step('support')
    spec = importlib.util.spec_from_file_location('private_pairing', RELEASE/'pairing_support.py')
    pairing = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(pairing)
    step('terminal')
    pairing.protect()
    pairing.require(len(sys.argv) == 1)
    validate_state(pairing)
    step('ready-confirm')
    write_all(1, '即将临时找回仍生效的旧手机条目；不会修改认证库、凭据或验证码尝试限制。\n'.encode())
    enter_only('确认当前是自己的私密终端，只按回车开始：')
    validate_state(pairing)  # 不让终端等待期间的旧状态成为解密依据。
    seed = read_current(pairing)
    validate_state(pairing)  # 显示前再确认尚未完成因子前滚。
    show_current(seed)
    step('complete')
    write_all(1, '旧条目已重新交付，尚未完成替换。现在用“Studiolo 旧因子（仅用于替换）”的手机码完成网页二次认证，再生成恢复码并用新条目完成恢复。不要贴终端内容。\n'.encode())


def failure_code(error):
    # 只用固定类别与整数errno；不输出异常正文、traceback、局部变量或子进程输出。
    category = 'interrupted' if isinstance(error, KeyboardInterrupt) else (
        'input-ended' if isinstance(error, EOFError) else (
        'os-error' if isinstance(error, OSError) else 'check-failed'))
    number = error.errno if isinstance(error, OSError) and isinstance(error.errno, int) else 0
    return 'T092/' + STAGE + '/' + category + '/' + str(number)


if __name__ == '__main__':
    try:
        main()
    except (Exception, KeyboardInterrupt) as error:
        write_all(2, ('旧条目再交付未确认完成；认证状态保持原样。仅将错误代码告诉 Codex：' + failure_code(error) + '\n').encode())
        raise SystemExit(1)
