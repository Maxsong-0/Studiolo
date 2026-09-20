#!/usr/bin/python3
"""T-093：只在108私有恢复副本撤销旧类型并验证401，不签发身份。"""
import ctypes
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import resource
import socket
import stat
import subprocess
import sys
import time

BASE = Path('/var/lib/studiolo-dr-tests/t093-20260920-1933')
RECOVERY = BASE / 'work/recovery'
BINARY = RECOVERY / 'bin/studiolo-agent'
PROBE = BASE / 'trusted-tools/check-agent.py'

def require(value):
    if not value:
        raise RuntimeError('rejected')

def main():
    require(os.getuid() == os.geteuid() == 0)
    os.umask(0o077)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    libc = ctypes.CDLL(None, use_errno=True)
    require(libc.prctl(4,0,0,0,0) == 0 and libc.prctl(3,0,0,0,0) == 0)
    require(libc.mlockall(1|2|4) == 0)
    require(os.readlink('/proc/self/ns/net') != os.readlink('/proc/1/ns/net'))
    for root in (BASE, RECOVERY, RECOVERY/'data', RECOVERY/'config'):
        item=root.lstat()
        require(stat.S_ISDIR(item.st_mode) and item.st_uid==0 and stat.S_IMODE(item.st_mode)==0o700)
    require(hashlib.sha256(BINARY.read_bytes()).hexdigest()=='c2af4ecf8af61ccfa2a32ec1e617a928fe40e2966ba7fe6aa67faa6a190e9b34')
    def command(args):
        return subprocess.run([str(x) for x in args],check=True,capture_output=True,text=True,timeout=30,
                              env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'})
    before=command([BINARY,'admin','types','--data',RECOVERY/'data']).stdout.splitlines()
    types=[]
    for line in before:
        fields=line.split('\t');require(len(fields)==3 and re.fullmatch('[a-z][a-z0-9-]{0,63}',fields[0]))
        require(fields[2]=='有效');types.append(fields[0])
    require(types and set(types)=={f.stem for f in (RECOVERY/'config').glob('*.token')})
    for name in types:
        command([BINARY,'admin','type-revoke','--data',RECOVERY/'data','--type',name])
    after=command([BINARY,'admin','types','--data',RECOVERY/'data']).stdout.splitlines()
    require(len(after)==len(types) and all(line.split('\t')[-1]=='已吊销' for line in after))
    spec=importlib.util.spec_from_file_location('t093_probe',PROBE)
    probe=importlib.util.module_from_spec(spec);spec.loader.exec_module(probe)
    server=subprocess.Popen([str(BINARY),'serve','--replica-root',str(BASE/'offline/replica'),
                             '--allow','项目/Studiolo','--allow','_规则','--data',str(RECOVERY/'data'),
                             '--policy',str(RECOVERY/'policy'),'--port','4493'],
                            env={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'},stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    try:
        deadline=time.monotonic()+15
        while True:
            require(server.poll() is None)
            try:
                with socket.create_connection(('127.0.0.1',4493),timeout=0.2):pass
                break
            except OSError:
                require(time.monotonic()<deadline);time.sleep(0.1)
        for name in types:
            path=RECOVERY/'config'/(name+'.token');item=path.lstat()
            require(stat.S_ISREG(item.st_mode) and item.st_uid==0 and stat.S_IMODE(item.st_mode)==0o600 and item.st_nlink==1)
            token=path.read_text().strip()
            probe.check('http://127.0.0.1:4493/mcp',token,'revoked')
            del token
        receipt={'network_namespace_isolated':True,'runtime_started':True,'types_revoked':len(types),
                 'original_tokens_http_401':len(types),'new_identity_issued':False,'positive_supply_check':False,
                 'production_modified':False,'backup_original_modified':False}
        (BASE/'agent-negative-acceptance.json').write_text(json.dumps(receipt,indent=2)+'\n')
        print(json.dumps(receipt))
    finally:
        server.terminate()
        try:server.wait(timeout=10)
        except subprocess.TimeoutExpired:server.kill();server.wait(timeout=5)

if __name__=='__main__':
    try:main()
    except BaseException as error:
        print(json.dumps({'status':'failed','error_class':type(error).__name__}))
        sys.exit(1)
