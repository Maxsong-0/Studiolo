#!/usr/bin/python3
"""不读取Tunnel凭据；验证目标系统单元隔离环境可连接私有Unix入口。"""
import http.client
import json
import os
from pathlib import Path
import socket
import stat
import sys

assert os.getuid() == os.geteuid() == 1000
assert os.getgid() == os.getegid() == 1000
userns = Path('/proc/self/ns/user').readlink()
# 初始命名空间由root监督进程读取后传入；目标UID不能可靠检查PID1的proc链接。
assert len(sys.argv) == 2 and sys.argv[1].startswith('user:[') and sys.argv[1].endswith(']')
initial = Path(sys.argv[1])
profile = Path('/proc/self/attr/current').read_text().strip()
print(json.dumps({'namespace':str(userns), 'initial_namespace':str(initial),
                  'uid_map':Path('/proc/self/uid_map').read_text().strip(),
                  'profile':profile}), flush=True)
assert profile == 'unconfined'
assert userns == initial
for name, mode in [('/run/studiolo-human', 0o700), ('/run/studiolo-human/http.sock', 0o600)]:
    metadata = Path(name).lstat()
    assert metadata.st_uid == 1000 and stat.S_IMODE(metadata.st_mode) == mode

class UnixHTTP(http.client.HTTPConnection):
    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(5)
        self.sock.connect('/run/studiolo-human/http.sock')

connection = UnixHTTP('studiolo.maxsong.me')
connection.request('GET', '/', headers={'Host': 'studiolo.maxsong.me'})
response = connection.getresponse()
response.read()
connection.close()
assert response.status == 401
print(json.dumps({'initial_user_namespace': userns == initial, 'uid': os.getuid(), 'unix_connect': 'passed', 'unauthenticated_status': response.status, 'credential_accessed': False}))
