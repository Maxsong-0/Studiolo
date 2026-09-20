#!/usr/bin/python3
"""使用同一共同停写快照重封装修正后的恢复工具；不重新停止生产。"""
import ctypes
import hashlib
import json
import os
from pathlib import Path
import resource
import runpy
import stat
import subprocess
import sys

CASE=Path('/var/lib/studiolo-dr-export/t094-20260920-ql5a20r3')
STAGE=Path('/home/maxsong/studiolo-t094-stage')


def require(ok):
    if not ok:raise ValueError('rejected')


def digest(p):
    h=hashlib.sha256()
    with p.open('rb') as f:
        for chunk in iter(lambda:f.read(1048576),b''):h.update(chunk)
    return h.hexdigest()


def main():
    require(os.getuid()==os.geteuid()==0 and sys.platform=='linux')
    require(sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode)
    os.umask(0o077);resource.setrlimit(resource.RLIMIT_CORE,(0,0))
    libc=ctypes.CDLL(None,use_errno=True)
    require(libc.prctl(4,0,0,0,0)==0 and libc.prctl(3,0,0,0,0)==0)
    require(digest(STAGE/'checksums.json')=='9d6c797674dfdab8069f9fe40d5453dc31f17bf104b5f1c61755c61daec7e956')
    require(digest(STAGE/'full_export.py')=='64843d84feb2c5d8156b2fe0b70d0169b6b2ffbf726eeff111f720e5f1bb1b8f')
    old=json.loads((CASE/'receipt.json').read_bytes())
    require(old['snapshot_id']==CASE.name and old['schema_version']==2)
    require(digest(CASE/'full-offline.tar.age')==old['cipher_sha256']=='996c47d83ccea6f7c85992be95b582179c146f54792cf911fd21fe25535a975a')
    require(digest(CASE/'offline/manifest.json')==old['manifest_sha256'])
    # STAGE摘要由启动调用者固定验证；这里只加载独立受信任代码，不加载恢复包程序。
    module=runpy.run_path(str(STAGE/'full_export.py'))
    module['check_running']()
    repack=CASE/'repack-r2';repack.mkdir(mode=0o700)
    (repack/'INCOMPLETE').write_text('repacking\n')
    tools=module['copy_tools'](repack)
    module['collect_runtime'](repack,tools)
    run=module['run']
    run(['python3','-I','-B','-S',tools/'pack_offline.py','verify',CASE/'offline','--tools',tools],stdout=subprocess.DEVNULL)
    run(['python3','-I','-B','-S',tools/'pack_offline.py','pack','--recovery',CASE/'recovery',
        '--replica',CASE/'frozen-replica','--human',CASE/'human','--tools',tools,
        '--runtime-libraries',repack/'runtime-libraries.txt','--os-release',repack/'os-release',
        '--output',repack/'offline'],stdout=subprocess.DEVNULL)
    partial=CASE/'full-offline-r2.tar.age.partial'
    with partial.open('xb') as output:
        archive=subprocess.Popen(['tar','--format=pax','-C',str(repack/'offline'),'-cf','-','.'],
            stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
        try:
            run(['/usr/bin/age','-R',tools/'recipient.txt'],stdin=archive.stdout,stdout=output)
        finally:
            archive.stdout.close()
            try:status=archive.wait(timeout=30)
            except subprocess.TimeoutExpired:archive.kill();archive.wait(timeout=5);raise
        require(status==0);output.flush();os.fsync(output.fileno())
    cipher=CASE/'full-offline-r2.tar.age';os.link(partial,cipher);partial.unlink()
    receipt=dict(old,tool_revision=2,cipher_file=cipher.name,cipher_sha256=digest(cipher),cipher_bytes=cipher.stat().st_size,
        manifest_sha256=digest(repack/'offline/manifest.json'),supersedes_cipher_sha256=old['cipher_sha256'],
        refrozen=False,original_snapshot_reused=True)
    module['write'](CASE/'receipt-r2.json',receipt)
    (repack/'INCOMPLETE').unlink()
    fd=os.open(CASE,os.O_RDONLY|os.O_DIRECTORY);os.fsync(fd);os.close(fd)
    module['check_running']()
    print(json.dumps(receipt,sort_keys=True))

if __name__=='__main__':
    try:main()
    except BaseException as error:
        tb=error.__traceback__;lines=[]
        while tb:lines.append(tb.tb_lineno);tb=tb.tb_next
        print(json.dumps({'status':'not_confirmed','error_class':type(error).__name__,'lines':lines}))
        sys.exit(1)
