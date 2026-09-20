#!/usr/bin/python3
"""T-092：v2 已生效之后部署策略页修复；不改认证配置和认证库。"""
import datetime
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import subprocess
import tempfile
import time

RELEASE = Path('/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r7')
CONFIG = Path('/home/maxsong/.config/studiolo-human/owner.json')
UNIT = Path('/etc/systemd/system/studiolo-human.service')
AUTH = Path('/home/maxsong/.local/share/studiolo-human-auth/auth.sqlite3')
SOCKET = Path('/run/studiolo-human/http.sock')
MANIFEST = '6b30030657a0807ed50abac5a106b38fa3b03602b6ff2e0de04071636aa0266d'
OLD_UNIT = 'e01aaa2a0f0f1c36d323df208614847f530abf77c27ac5f4549888a3ce7761b8'
NEW_UNIT = '66d3689f8d73a42edf1ba56364cc8a3212b4533ce91ca8bffe09d7455be5d847'
STAGE = 'preflight'
RECORD = None

def require(value):
    if not value:
        raise RuntimeError('rejected')

def command(args):
    return subprocess.run(args, check=True, capture_output=True, text=True,
                          env={'PATH':'/usr/sbin:/usr/bin:/sbin:/bin','LANG':'C.UTF-8'})

def regular(path, uid, mode):
    item = path.lstat()
    require(stat.S_ISREG(item.st_mode) and item.st_uid == uid and item.st_nlink == 1)
    require(stat.S_IMODE(item.st_mode) == mode)
    for parent in path.parents:
        item = parent.lstat()
        require(stat.S_ISDIR(item.st_mode) and item.st_uid in (0,uid))
        require(stat.S_IMODE(item.st_mode) & 0o7022 == 0)

def binding():
    regular(AUTH,1000,0o600)
    with sqlite3.connect('file:'+str(AUTH)+'?mode=ro',uri=True) as db:
        db.execute('PRAGMA trusted_schema=OFF')
        db.execute('PRAGMA query_only=ON')
        require(db.execute('PRAGMA user_version').fetchone() == (4,))
        require(db.execute('SELECT seed_ref,enrollment_json IS NOT NULL FROM totp_factor').fetchall()
                == [('studiolo-human-totp-v2',1)])
        for table in ('totp_pending','recovery_pending'):
            require(db.execute('SELECT count(*) FROM '+table).fetchone() == (0,))
        require(db.execute('SELECT change_json IS NOT NULL FROM recovery_control').fetchone() == (1,))

def atomic_write(path,data,uid,gid,mode):
    descriptor,name=tempfile.mkstemp(prefix='.t092-forward-',dir=path.parent)
    try:
        with os.fdopen(descriptor,'wb') as output:
            os.fchmod(output.fileno(),mode)
            os.fchown(output.fileno(),uid,gid)
            output.write(data);output.flush();os.fsync(output.fileno())
        os.replace(name,path)
        parent=os.open(path.parent,os.O_RDONLY|os.O_DIRECTORY)
        try:os.fsync(parent)
        finally:os.close(parent)
    finally:
        if os.path.lexists(name):os.unlink(name)

def main():
    global STAGE,RECORD
    require(os.getuid()==os.geteuid()==0)
    os.umask(0o077)
    manifest_bytes=(RELEASE/'manifest.json').read_bytes()
    require(hashlib.sha256(manifest_bytes).hexdigest()==MANIFEST)
    manifest=json.loads(manifest_bytes)
    for relative,expected in manifest['files'].items():
        path=Path(relative)
        require(not path.is_absolute() and '..' not in path.parts)
        item=RELEASE/path
        metadata=item.lstat()
        require(stat.S_ISREG(metadata.st_mode) and metadata.st_nlink==1)
        require(metadata.st_uid==1000 and stat.S_IMODE(metadata.st_mode)&0o7022==0)
        content=item.read_bytes()
        require(len(content)==expected['bytes'])
        require(hashlib.sha256(content).hexdigest()==expected['sha256'])
    regular(CONFIG,1000,0o600)
    before=CONFIG.read_bytes();config=json.loads(before)
    require(config['seed_ref']=='studiolo-human-totp-v2')
    require(config['auth_directory']==str(AUTH.parent))
    require(config['audit_directory']=='/home/maxsong/.local/share/studiolo-human-audit')
    for directory,name in (('studiolo-human-audit','audit'),('studiolo-agent-policy','policy')):
        regular(Path('/home/maxsong/.local/share')/directory/(name+'.sqlite3'),1000,0o600)
    regular(Path('/etc/credstore.encrypted/studiolo-human-totp-v2'),0,0o600)
    regular(UNIT,0,0o644)
    require(hashlib.sha256(UNIT.read_bytes()).hexdigest()==OLD_UNIT)
    new_unit=(RELEASE/'studiolo-human.service').read_bytes()
    require(hashlib.sha256(new_unit).hexdigest()==NEW_UNIT)
    command(['systemd-analyze','verify',str(RELEASE/'studiolo-human.service')])
    binding()
    require(command(['systemctl','is-active','studiolo-human.service']).stdout.strip()=='active')
    RECORD=Path(tempfile.mkdtemp(prefix='t092-web-r7-',dir='/var/lib/studiolo-deployments'))
    (RECORD/'owner.before.json').write_bytes(before)
    (RECORD/'human.before.service').write_bytes(UNIT.read_bytes())
    receipt={'manifest_sha256':MANIFEST,'files_verified':len(manifest['files']),
             'auth_database_modified_by_deployer':False,'owner_config_modified':False,'stop_requested_at':datetime.datetime.now(datetime.timezone.utc).isoformat()}
    (RECORD/'started.json').write_text(json.dumps(receipt))
    STAGE='stop-r5'
    command(['systemctl','stop','studiolo-human.service'])
    require(command(['systemctl','show','studiolo-human.service','--property=ActiveState','--value']).stdout.strip()=='inactive')
    require(not os.path.lexists(SOCKET))
    binding()
    require(CONFIG.read_bytes()==before)
    STAGE='forward-unit'
    atomic_write(UNIT,new_unit,0,0,0o644)
    command(['systemctl','daemon-reload'])
    STAGE='start-r7'
    command(['systemctl','start','studiolo-human.service'])
    deadline=time.monotonic()+15
    while not SOCKET.exists() and time.monotonic()<deadline:
        time.sleep(0.2)
    require(SOCKET.exists())
    STAGE='accept-r7'
    properties=command(['systemctl','show','studiolo-human.service',
                        '--property=ActiveState,SubState,MainPID,NRestarts']).stdout
    properties=dict(line.split('=',1) for line in properties.splitlines())
    require(properties['ActiveState']=='active' and properties['SubState']=='running' and properties['NRestarts']=='0')
    pid=int(properties['MainPID'])
    require(os.readlink('/proc/'+str(pid)+'/exe')==str(RELEASE/'bin/studiolo-local-server'))
    credential_dir=Path('/proc/'+str(pid)+'/root/run/credentials/studiolo-human.service')
    require(sorted(p.name for p in credential_dir.iterdir())==['totp-next'])
    for path,mode in ((SOCKET.parent,0o700),(SOCKET,0o600)):
        item=path.lstat();require(item.st_uid==1000 and stat.S_IMODE(item.st_mode)==mode)
    denials=[]
    for method,path in (('GET','/'),('GET','/api/v1/session'),('POST','/api/v1/sessions'),
                        ('POST','/api/v1/auth/step-up'),('POST','/api/v1/auth/recovery-codes'),
                        ('POST','/api/v1/auth/factor/recover')):
        args=['sudo','-n','-u','maxsong','curl','--silent','--show-error','--max-time','5','--unix-socket',str(SOCKET),
              '--output','/dev/null','--write-out','%{http_code}','--header','Host: studiolo.maxsong.me',
              '--request',method,'http://localhost'+path]
        if method=='POST':args+=['--header','Content-Type: application/json','--data','{}']
        code=command(args).stdout
        require(code=='401');denials.append({'method':method,'path':path,'status':401})
    binding()
    require(CONFIG.read_bytes()==before)
    command(['systemctl','enable','studiolo-human.service'])
    require(command(['systemctl','is-enabled','studiolo-human.service']).stdout.strip()=='enabled')
    receipt.update({'ready_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    'status':'accepted','pid':pid,'sole_credential':'totp-next','unit_sha256':NEW_UNIT,'denials':denials})
    (RECORD/'accepted.json').write_text(json.dumps(receipt,indent=2))
    print(json.dumps({'deployment_record':str(RECORD),**receipt}))

if __name__=='__main__':
    try:main()
    except BaseException as error:
        status={'status':'failed','stage':STAGE,'error_class':type(error).__name__,
                'deployment_record':str(RECORD) if RECORD else None,
                'automatic_auth_or_config_rollback':False}
        if RECORD:
            (RECORD/'failed.json').write_text(json.dumps(status))
        print(json.dumps(status))
        raise SystemExit(1)
