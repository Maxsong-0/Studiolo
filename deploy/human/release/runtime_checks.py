"""固定human服务运行核验；不读取进程环境或凭据内容。"""
import os
from pathlib import Path
import stat
import subprocess
import time
import release_common as c

SERVICE = 'studiolo-human.service'
UNIT = Path('/etc/systemd/system') / SERVICE
SOCKET = Path('/run/studiolo-human/http.sock')
OLD_UNIT = '66d3689f8d73a42edf1ba56364cc8a3212b4533ce91ca8bffe09d7455be5d847'
OLD_BINARY = '8b5f1aa5643b7e9c8934760e307c84b365babd91cd953952ede51d96805bae0a'
ENV = {'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'LANG': 'C.UTF-8'}
ENDPOINTS = (('GET', '/'), ('GET', '/api/v1/session'), ('POST', '/api/v1/sessions'),
             ('POST', '/api/v1/auth/step-up'), ('POST', '/api/v1/auth/recovery-codes'),
             ('POST', '/api/v1/auth/factor/recover'))


def command(args, timeout=30):
    return subprocess.run([str(x) for x in args], check=True, capture_output=True,
                          text=True, timeout=timeout, env=ENV, stdin=subprocess.DEVNULL)


def properties():
    raw = command(['systemctl', 'show', SERVICE,
                   '--property=ActiveState,SubState,MainPID,NRestarts,FragmentPath,DropInPaths,UnitFileState'], 2).stdout
    values = dict(line.split('=', 1) for line in raw.splitlines())
    c.require(values['FragmentPath'] == str(UNIT) and values['DropInPaths'] == '')
    c.require(values['UnitFileState'] == 'enabled')
    return values


def stopped():
    values = properties()
    c.require(values['ActiveState'] in ('inactive', 'failed') and values['MainPID'] == '0')
    c.require(not os.path.lexists(SOCKET))


def wait_program(binary, *, deadline_seconds=20, get_properties=properties,
                 readlink=os.readlink, sleep=time.sleep, monotonic=time.monotonic,
                 socket_exists=lambda: os.path.lexists(SOCKET)):
    deadline = monotonic() + deadline_seconds
    while True:
        values = get_properties()
        pid = int(values['MainPID'])
        actual = None
        if pid > 0:
            try:
                actual = readlink('/proc/' + str(pid) + '/exe')
            except FileNotFoundError:
                pass
        if (values['ActiveState'] == 'active' and values['SubState'] == 'running'
                and values['NRestarts'] == '0' and actual == str(binary) and socket_exists()):
            return pid
        c.require(monotonic() < deadline and values['NRestarts'] == '0')
        sleep(0.1)


def ready(binary, digest):
    pid = wait_program(binary)
    c.require(c.sha(c.read(binary, c.UID, 0o755)) == digest)
    opened, supplied = os.stat('/proc/' + str(pid) + '/exe'), binary.stat()
    c.require((opened.st_dev, opened.st_ino) == (supplied.st_dev, supplied.st_ino))
    credentials = Path('/proc') / str(pid) / 'root/run/credentials' / SERVICE
    c.require(sorted(p.name for p in credentials.iterdir()) == ['totp-next'])
    for path, mode, directory in ((SOCKET.parent, 0o700, True), (SOCKET, 0o600, False)):
        s = path.lstat()
        c.require(s.st_uid == c.UID and stat.S_IMODE(s.st_mode) == mode)
        c.require(stat.S_ISDIR(s.st_mode) if directory else stat.S_ISSOCK(s.st_mode) and s.st_nlink == 1)
    c.require(properties()['MainPID'] == str(pid))
    return pid


def denials():
    for method, path in ENDPOINTS:
        args = ['sudo', '-n', '-u', 'maxsong', 'curl', '--silent', '--show-error', '--max-time', '3',
                '--unix-socket', SOCKET, '--output', '/dev/null', '--write-out', '%{http_code}',
                '--header', 'Host: studiolo.maxsong.me', '--request', method, 'http://localhost' + path]
        if method == 'POST':
            args += ['--header', 'Content-Type: application/json', '--data', '{}']
        c.require(command(args, 5).stdout == '401')
