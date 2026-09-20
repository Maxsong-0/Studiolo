#!/usr/bin/python3
"""T-094受控共同停写导出；只在169已核验装配执行，不对任何生产身份撤权。"""
import ctypes
import datetime
import fcntl
import hashlib
import json
import os
from pathlib import Path
import resource
import shutil
import stat
import subprocess
import sys
import tempfile
import time

BASE = Path('/var/lib/studiolo-dr-export')
STAGE = Path('/home/maxsong/studiolo-t094-stage')
HOME = Path('/home/maxsong')
DATA = HOME / '.local/share/studiolo-agent'
CONFIG = HOME / '.config/studiolo-agent'
POLICY = HOME / '.local/share/studiolo-agent-policy'
REPLICA = HOME / '.local/share/studiolo-agent-replica'
AUTH = HOME / '.local/share/studiolo-human-auth'
AUDIT = HOME / '.local/share/studiolo-human-audit'
OWNER = HOME / '.config/studiolo-human/owner.json'
RELEASE = HOME / '.local/share/studiolo-human-releases/m4-t092-20260920-r7'
BINARY = HOME / '.local/share/studiolo-agent-releases/m5m6-338a103-20260920/bin/studiolo-agent'
CREDENTIAL = Path('/etc/credstore.encrypted/studiolo-human-totp-v2')
UNITS = ('studiolo-human.service', 'studiolo-agent.service')
EXPECTED_AGENT = 'ca14d2df43d14c13293fc8b76d7870ddbf86fe59826921c6a372bd554341dd12'
EXPECTED_HUMAN = '8b5f1aa5643b7e9c8934760e307c84b365babd91cd953952ede51d96805bae0a'
EXPECTED_RELEASE = '6b30030657a0807ed50abac5a106b38fa3b03602b6ff2e0de04071636aa0266d'


def require(ok):
    if not ok:
        raise ValueError('precondition')


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def run(args, **kwargs):
    if kwargs.pop('capture_output', False):
        kwargs['stdout'] = subprocess.PIPE
    kwargs.setdefault('stdin', subprocess.DEVNULL)
    return subprocess.run(list(map(str, args)), check=True,
                          stderr=subprocess.DEVNULL, timeout=120, **kwargs)


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b''):
            result.update(chunk)
    return result.hexdigest()


def metadata(path, uid, directory=False, private=True):
    current = path
    while True:
        item = current.lstat()
        require(item.st_uid in (0, uid) and not item.st_mode & 0o7022)
        require(stat.S_ISDIR(item.st_mode) if current != path or directory else stat.S_ISREG(item.st_mode))
        if current == path:
            require(item.st_uid == uid)
            if not directory:
                require(item.st_nlink == 1)
            if private:
                require(stat.S_IMODE(item.st_mode) == (0o700 if directory else 0o600))
        if current == Path('/'):
            return
        current = current.parent


def write(path, value):
    with path.open('x', encoding='utf-8') as stream:
        stream.write(json.dumps(value, sort_keys=True, indent=2) + '\n')
        stream.flush()
        os.fsync(stream.fileno())


def service(unit):
    raw = run(['systemctl', 'show', unit, '--property=ActiveState,MainPID'], capture_output=True).stdout.decode()
    return dict(line.split('=', 1) for line in raw.splitlines())


def check_running():
    for unit, expected in zip(UNITS, (RELEASE / 'bin/studiolo-local-server', BINARY)):
        value = service(unit)
        require(value['ActiveState'] == 'active' and int(value['MainPID']) > 0)
        require(os.readlink('/proc/' + value['MainPID'] + '/exe') == str(expected))
    require(digest(BINARY) == EXPECTED_AGENT)
    require(digest(RELEASE / 'bin/studiolo-local-server') == EXPECTED_HUMAN)
    require(digest(RELEASE / 'manifest.json') == EXPECTED_RELEASE)


def wait_running():
    # Type=simple 的 active 先于 launcher exec；只在原服务启动后有界等待真实程序就绪。
    deadline = time.monotonic() + 10
    while True:
        try:
            check_running()
            return
        except (OSError, ValueError):
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.05)


def no_open_stores():
    # 所有已知写者退出后拒绝仍持有库的进程，不打印其命令行或FD内容。
    prefixes = tuple(str(path) + '/' for path in (DATA, POLICY, AUTH, AUDIT))
    for process in Path('/proc').iterdir():
        if not process.name.isdigit() or int(process.name) == os.getpid():
            continue
        try:
            for descriptor in (process / 'fd').iterdir():
                try:
                    require(not os.readlink(descriptor).startswith(prefixes))
                except FileNotFoundError:
                    pass
        except (FileNotFoundError, ProcessLookupError):
            pass


def copy_tools(case):
    metadata(STAGE, 1000, directory=True)
    expected = json.loads((STAGE / 'checksums.json').read_bytes())
    require(type(expected) is dict and 10 <= len(expected) <= 128)
    actual = {str(p.relative_to(STAGE)) for p in STAGE.rglob('*') if p.is_file()}
    require(actual == set(expected) | {'checksums.json'})
    tools = case / 'tools'
    tools.mkdir(mode=0o700)
    for name, sha in sorted(expected.items()):
        require(type(name) is str and all(part and not part.startswith('.') for part in name.split('/')))
        require(type(sha) is str and len(sha) == 64)
        source = STAGE / name
        metadata(source, 1000, private=False)
        require(digest(source) == sha)
        destination = tools / name
        destination.parent.mkdir(parents=True, mode=0o700, exist_ok=True)
        shutil.copyfile(source, destination)
        destination.chmod(0o700 if name == 'factor-pipe' else 0o600)
        require(digest(destination) == sha == digest(source))
    return tools


def collect_runtime(case, tools):
    """把实际动态库和程序来源一并封装；不依赖旧机器仍然存在。"""
    runtime = tools / 'runtime'
    runtime.mkdir(mode=0o700)
    provenance, readings = {}, []
    binaries = (BINARY, RELEASE / 'bin/studiolo-local-server',
                tools / 'restore-runtime/studiolo-auth-admin',
                tools / 'restore-runtime/studiolo-local-server')
    for binary in binaries:
        result = run(['ldd', binary], capture_output=True).stdout.decode()
        require('not found' not in result)
        readings.append(str(binary) + '\n' + result)
        for line in result.splitlines():
            paths = [word for word in line.split() if word.startswith('/')]
            if not paths:
                continue
            source = Path(paths[0]).resolve(strict=True)
            metadata(source, 0, private=False)
            sha = digest(source)
            target = runtime / source.name
            if target.exists():
                require(digest(target) == sha)
            else:
                shutil.copyfile(source, target)
                target.chmod(0o600)
            provenance[source.name] = {'source': str(source), 'sha256': sha}
    require(len(provenance) >= 3)
    write(tools / 'runtime-provenance.json', provenance)
    (case / 'runtime-libraries.txt').write_text('\n'.join(readings))
    shutil.copyfile('/usr/lib/os-release', case / 'os-release')


def freeze(case, tools):
    arguments = ['--data', DATA, '--config', CONFIG, '--binary', BINARY,
                 '--unit', '/etc/systemd/system/studiolo-agent.service', '--policy', POLICY]
    recovery = ['/usr/bin/python3', '-I', '-B', tools / 'm4_recovery.py']
    run(recovery + ['preflight', *arguments], stdout=subprocess.DEVNULL)
    lock = os.open(REPLICA / 'sync.lock', os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC)
    try:
        item = os.fstat(lock)
        require(item.st_uid == 1000 and stat.S_ISREG(item.st_mode) and item.st_nlink == 1)
        require(stat.S_IMODE(item.st_mode) == 0o600)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        check_running()
        started = now()
        try:
            run(['systemctl', 'stop', *UNITS], stdout=subprocess.DEVNULL)
            require(all(service(unit)['ActiveState'] == 'inactive' for unit in UNITS))
            no_open_stores()
            run(recovery + ['create', *arguments, '--output', case / 'recovery'], stdout=subprocess.DEVNULL)
            shutil.copytree(REPLICA, case / 'frozen-replica', symlinks=True)
            recipient = (tools / 'recipient.txt').read_text().strip()
            require(recipient.startswith('age1') and len(recipient) == 62)
            run(['/usr/bin/python3', '-I', '-B', '-S', tools / 'portable_factor.py', 'export',
                 '--credential', CREDENTIAL, '--recipient', recipient, '--filter-bin', tools / 'factor-pipe',
                 '--output', case / 'portable-factor.age'], stdout=subprocess.DEVNULL)
            run(['/usr/bin/python3', '-I', '-B', '-S', tools / 'human_snapshot.py', 'create',
                 '--auth-dir', AUTH, '--audit-dir', AUDIT, '--config', OWNER, '--release', RELEASE,
                 '--portable-factor', case / 'portable-factor.age', '--snapshot-id', case.name,
                 '--output', case / 'human'], stdout=subprocess.DEVNULL)
        finally:
            # 仅恢复本操作原先确认active的两个单元，不回滚数据库、不清理socket。
            run(['systemctl', 'start', 'studiolo-agent.service', 'studiolo-human.service'], stdout=subprocess.DEVNULL)
            wait_running()
        return {'stopped_at': started, 'restarted_at': now()}
    finally:
        os.close(lock)


def main():
    require(sys.argv[1:] == ['--execute'] and sys.platform == 'linux')
    require(os.getuid() == os.geteuid() == 0 and sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode)
    os.umask(0o077)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    libc = ctypes.CDLL(None, use_errno=True)
    require(libc.prctl(4, 0, 0, 0, 0) == 0 and libc.prctl(3, 0, 0, 0, 0) == 0)
    BASE.mkdir(mode=0o700, exist_ok=True)
    metadata(BASE, 0, directory=True)
    check_running()
    case = Path(tempfile.mkdtemp(prefix='t094-20260920-', dir=BASE))
    # tempfile可能含下划线，而协议快照ID仅接受字母数字与连字符。
    renamed = case.with_name(case.name.replace('_', '-'))
    if renamed != case:
        require(not renamed.exists())
        case.rename(renamed)
        case = renamed
    (case / 'INCOMPLETE').write_text('incomplete\n')
    tools = copy_tools(case)
    collect_runtime(case, tools)
    timing = freeze(case, tools)
    run(['/usr/bin/python3', '-I', '-B', '-S', tools / 'pack_offline.py', 'pack',
         '--recovery', case / 'recovery', '--replica', case / 'frozen-replica', '--human', case / 'human',
         '--tools', tools, '--runtime-libraries', case / 'runtime-libraries.txt',
         '--os-release', case / 'os-release', '--output', case / 'offline'], stdout=subprocess.DEVNULL)
    partial = case / 'full-offline.tar.age.partial'
    with partial.open('xb') as output:
        archive = subprocess.Popen(['tar', '--format=pax', '-C', str(case / 'offline'), '-cf', '-', '.'],
                                   stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        try:
            run(['/usr/bin/age', '-R', tools / 'recipient.txt'], stdin=archive.stdout, stdout=output)
        finally:
            archive.stdout.close()
            try:
                status = archive.wait(timeout=30)
            except subprocess.TimeoutExpired:
                archive.kill()
                archive.wait(timeout=5)
                raise
        require(status == 0)
        output.flush()
        os.fsync(output.fileno())
    cipher = case / 'full-offline.tar.age'
    os.link(partial, cipher)
    partial.unlink()
    receipt = {'schema_version': 2, 'snapshot_id': case.name, **timing,
               'human_auth_included': True, 'portable_factor_included': True,
               'factor_binding_restore_checked': False, 'restored_credentials_invalidated': False,
               'canonical_whole_vault_included': False, 'cipher_sha256': digest(cipher),
               'cipher_bytes': cipher.stat().st_size, 'manifest_sha256': digest(case / 'offline/manifest.json'),
               'agent_binary_sha256': EXPECTED_AGENT, 'human_binary_sha256': EXPECTED_HUMAN}
    write(case / 'receipt.json', receipt)
    fd = os.open(case, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
        (case / 'INCOMPLETE').unlink()
        os.fsync(fd)
    finally:
        os.close(fd)
    print(json.dumps({'export': str(case), **receipt}, sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError):
        print('共同停写导出未确认完成；保留本批状态，先核对两个原服务，再检查非秘密回执。', file=sys.stderr)
        sys.exit(1)
