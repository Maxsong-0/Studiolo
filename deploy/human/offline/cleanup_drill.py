import datetime,json,os,shutil,stat,subprocess
from pathlib import Path
base=Path('/var/lib/studiolo-dr-tests')
names=['t094-20260920-ql5a20r3','t094-20260920-ql5a20r3-r2','t094-fixtures-20260920-a','t094-tools-20260920-a','t094-tools-r2','t094-r2-binaries']
assert os.getuid()==os.geteuid()==0
roots=[base/name for name in names]
assert base.lstat().st_uid==0 and stat.S_IMODE(base.stat().st_mode)==0o700
old=json.loads((roots[0]/'offline/manifest.json').read_bytes())
new=json.loads((roots[1]/'offline/manifest.json').read_bytes())
for prefix in ('human/','recovery/','replica/'):
    assert {k:v for k,v in old['files'].items() if k.startswith(prefix)}=={k:v for k,v in new['files'].items() if k.startswith(prefix)}
for unit in ('studiolo-t094-human-restore','studiolo-t094-human-restore-r2','studiolo-t094-agent-restore','studiolo-t094-fixture-cli','studiolo-t094-fixture-cli-r2','studiolo-t094-fixture-pipe','studiolo-t094-fixture-crypto'):
    r=subprocess.run(['systemctl','show',unit+'.service','--property=ActiveState,MainPID'],capture_output=True,text=True)
    status=dict(line.split('=',1) for line in r.stdout.splitlines() if '=' in line)
    assert status.get('ActiveState') in ('inactive','failed') and status.get('MainPID')=='0'
for root in roots:
    assert root.parent==base and root.lstat().st_uid==0 and stat.S_ISDIR(root.lstat().st_mode)
    device=root.stat().st_dev
    for node in [root,*root.rglob('*')]:
        s=node.lstat();assert s.st_uid==0 and s.st_dev==device
        assert stat.S_ISDIR(s.st_mode) or stat.S_ISREG(s.st_mode) and s.st_nlink==1
prefixes=tuple(str(root)+'/' for root in roots)
for proc in Path('/proc').iterdir():
    if not proc.name.isdigit() or int(proc.name)==os.getpid():continue
    try:
        nodes=[proc/'cwd',proc/'exe',*list((proc/'fd').iterdir())]
        for node in nodes:
            try:target=os.readlink(node)
            except (FileNotFoundError,ProcessLookupError):continue
            assert target not in [str(root) for root in roots] and not target.startswith(prefixes)
    except (FileNotFoundError,ProcessLookupError):pass
for root in roots:shutil.rmtree(root)
assert all(not root.exists() for root in roots)
assert not Path('/var/lib/systemd/credential.secret').exists()
print(json.dumps({'temporary_plaintext_removed':True,'private_dr_keys_removed':True,'shared_host_key_unchanged_absent':True,'deleted_directories':names,'frozen_components_identical_across_tool_revision':True,'removed_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'physical_erasure_claimed':False},sort_keys=True))
