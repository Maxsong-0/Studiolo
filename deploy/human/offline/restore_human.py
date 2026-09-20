#!/usr/bin/python3
"""在独立netns内验证已认证的新恢复副本；只让原生进程接收因子明文。"""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import resource
import shutil
import sqlite3
import stat
import subprocess
import sys

BASE=Path('/var/lib/studiolo-dr-tests')
TOOLS=BASE/'t094-tools-r2'
RUN='t094-20260920-ql5a20r3-r2'
CASE=BASE/RUN
SOURCE=CASE/'offline/human'
WORK=CASE/'human-work'
KEYS=CASE/'keys'
BINARY=TOOLS/'restore-runtime/studiolo-auth-admin'
ENV={'PATH':'/usr/bin:/bin','LANG':'C.UTF-8'}


def require(ok):
    if not ok:raise ValueError('rejected')


def digest(path):
    h=hashlib.sha256()
    with path.open('rb') as f:
        for chunk in iter(lambda:f.read(1048576),b''):h.update(chunk)
    return h.hexdigest()


def metadata(path,mode,directory=False):
    s=path.lstat()
    require(s.st_uid==0 and stat.S_IMODE(s.st_mode)==mode)
    require(stat.S_ISDIR(s.st_mode) if directory else stat.S_ISREG(s.st_mode) and s.st_nlink==1)
    return (s.st_dev,s.st_ino,s.st_size,s.st_mtime_ns,s.st_ctime_ns)


def database(path):
    connection=sqlite3.connect('file:'+str(path)+'?mode=ro',uri=True)
    connection.execute('PRAGMA query_only=ON')
    connection.execute('PRAGMA trusted_schema=OFF')
    result={}
    try:
        require(connection.execute('PRAGMA quick_check(1)').fetchall()==[('ok',)])
        for (name,) in connection.execute("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name"):
            require(re.fullmatch('[a-z_]{1,64}',name) is not None)
            result[name]=connection.execute('SELECT * FROM '+name+' ORDER BY rowid').fetchall()
    finally:connection.close()
    return result


def invalidate():
    credential=KEYS/'totp-next'
    hostkey=KEYS/'host-key'
    before=[metadata(credential,0o600),metadata(hostkey,0o400)]
    decrypted=subprocess.Popen(['/usr/bin/systemd-creds','decrypt','--name=totp-next','--newline=no',str(credential),'-'],
        stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,
        env=dict(ENV,SYSTEMD_CREDENTIAL_SECRET=str(hostkey)))
    try:
        response=subprocess.run([str(BINARY),'restore-invalidate','--config',str(WORK/'owner.json'),
            '--restore-root',str(WORK),'--restore-id',RUN],stdin=decrypted.stdout,
            stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,env=ENV,timeout=30)
        decrypted.stdout.close()
        require(decrypted.wait(timeout=5)==0)
        require(response.returncode==0 and len(response.stdout)<4096)
        result=json.loads(response.stdout)
        require(set(result)=={'epoch','revoked_sessions','cleared_l3','invalidated_recovery_codes','audit_confirmed'})
        require(result['audit_confirmed'] is True)
        require(before==[metadata(credential,0o600),metadata(hostkey,0o400)])
        return result
    finally:
        if decrypted.poll() is None:decrypted.kill();decrypted.wait(timeout=5)


def verify_change(before,after,report):
    changed={'auth_meta','sessions','audit_outbox','totp_grants','recovery_codes','recovery_control'}
    require(set(before)==set(after))
    require(all(before[name]==after[name] for name in before if name not in changed))
    original=list(before['auth_meta'][0]);original[3]+=1;original[5]+=2
    require(after['auth_meta']==[tuple(original)] and report['epoch']==original[3])
    expected=[];count=0
    for row in before['sessions']:
        row=list(row)
        if row[4] in ('active','pending'):row[4]='revoked';count+=1
        expected.append(tuple(row))
    require(after['sessions']==expected and report['revoked_sessions']==count)
    require(after['totp_grants']==[] and report['cleared_l3']==len(before['totp_grants']))
    require(after['recovery_codes']==[] and report['invalidated_recovery_codes']==len(before['recovery_codes']))
    control=list(before['recovery_control'][0]);control[2]=None;control[6]=0
    require(after['recovery_control']==[tuple(control)])
    old=before['audit_outbox'];new=after['audit_outbox']
    require(len(new)==len(old)+2 and new[:len(old)]==old)
    for row,phase in zip(new[len(old):],('intent','outcome')):
        require(row[1].endswith(':restore:'+RUN) and row[2]==phase and row[8] is not None)
        value=json.loads(row[3]);require(value['payload']['scope']=='restore')
        require(value['payload']['identity_version']==report['epoch'])


def main():
    require(sys.argv[1:]==['--execute'] and sys.platform=='linux' and os.getuid()==os.geteuid()==0)
    require(sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode)
    require(os.readlink('/proc/self/ns/net')!=os.readlink('/proc/1/ns/net'))
    os.umask(0o077);resource.setrlimit(resource.RLIMIT_CORE,(0,0))
    libc=ctypes.CDLL(None,use_errno=True)
    require(libc.prctl(4,0,0,0,0)==0 and libc.prctl(3,0,0,0,0)==0 and libc.mlockall(1|2|4)==0)
    for p in (BASE,CASE,SOURCE,TOOLS,KEYS):metadata(p,0o700,True)
    require(digest(BINARY)=='c7f96a83d80ea1128698495ff0aa7c13f1078b8ea683a3e7343edf345ecc9699')
    subprocess.run(['python3','-I','-B','-S',str(TOOLS/'pack_offline.py'),'verify',str(CASE/'offline'),'--tools',str(TOOLS)],
        check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=30)
    WORK.mkdir(mode=0o700)
    for label in ('auth','audit'):
        (WORK/label).mkdir(mode=0o700)
        source=SOURCE/label/(label+'.sqlite3');target=WORK/label/(label+'.sqlite3')
        metadata(source,0o600)
        shutil.copyfile(source,target);target.chmod(0o600)
        require(digest(source)==digest(target) and source.stat().st_ino!=target.stat().st_ino)
    config=json.loads((SOURCE/'owner.json').read_bytes())
    config['auth_directory']=str(WORK/'auth');config['audit_directory']=str(WORK/'audit')
    require(config['seed_ref']=='studiolo-human-totp-v2')
    (WORK/'owner.json').write_text(json.dumps(config,sort_keys=True)+'\n')
    before=database(WORK/'auth/auth.sqlite3');ledger_before=database(WORK/'audit/audit.sqlite3')
    report=invalidate()
    after=database(WORK/'auth/auth.sqlite3');ledger_after=database(WORK/'audit/audit.sqlite3')
    verify_change(before,after,report)
    events=ledger_before['audit_events'];new=ledger_after['audit_events']
    require(new[:len(events)]==events and len(new)==len(events)+2)
    require(invalidate()==report)
    require(database(WORK/'auth/auth.sqlite3')==after and database(WORK/'audit/audit.sqlite3')==ledger_after)
    subprocess.run(['python3','-I','-B','-S',str(TOOLS/'pack_offline.py'),'verify',str(CASE/'offline'),'--tools',str(TOOLS)],
        check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=30)
    receipt={'snapshot_id':'t094-20260920-ql5a20r3','restore_id':RUN,**report,'factor_v2_binding_verified':True,'factor_and_consumption_proofs_preserved':True,
        'clock_and_rate_budgets_preserved':True,'original_audit_prefix_preserved':True,'restore_audit_pair_verified':True,
        'same_restore_id_idempotent':True,'isolated_network_namespace':True,'original_bundle_unchanged':True,
        'production_auth_modified':False,'fresh_human_login_l3_tested':False,'runtime_public_listener_started':False}
    with (CASE/'human-restore-receipt.json').open('x') as f:
        f.write(json.dumps(receipt,sort_keys=True,indent=2)+'\n');f.flush();os.fsync(f.fileno())
    print(json.dumps(receipt,sort_keys=True))

if __name__=='__main__':
    try:main()
    except BaseException as error:
        # 只输出程序行号和异常类型，绝不输出异常数据、SQL行或子进程正文。
        tb=error.__traceback__;lines=[]
        while tb:
            lines.append(tb.tb_lineno);tb=tb.tb_next
        print(json.dumps({'status':'not_confirmed','error_class':type(error).__name__,'lines':lines}))
        sys.exit(1)
