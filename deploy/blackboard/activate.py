#!/usr/bin/python3
"""固定 r8 → board-r2 双服务切换；失败只恢复程序，不回滚数据库。"""
import datetime
import json
import os
from pathlib import Path
import re
import resource
import signal
import stat
import sys
import types

HERE = Path(os.path.abspath(__file__)).parent


def trusted_source(path):
    """先核验所有祖先和文件，再从同一受检 FD 编译，不跟随链接。"""
    for parent in path.parents:
        s = parent.lstat()
        if not stat.S_ISDIR(s.st_mode) or s.st_uid not in (0, 1000) or s.st_mode & 0o7022:
            raise ValueError('unsafe tools')
    before = path.lstat()
    if (not stat.S_ISREG(before.st_mode) or before.st_uid not in (0, 1000)
            or before.st_nlink != 1 or before.st_mode & 0o7022 or before.st_size > 1024 * 1024):
        raise ValueError('unsafe tools')
    def mark(s):
        return (s.st_dev, s.st_ino, s.st_mode, s.st_uid, s.st_gid, s.st_nlink,
                s.st_size, s.st_mtime_ns, s.st_ctime_ns)
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    try:
        if mark(os.fstat(fd)) != mark(before):
            raise ValueError('unsafe tools')
        with os.fdopen(fd, 'rb', closefd=False) as stream:
            data = stream.read(1024 * 1024 + 1)
        if (len(data) != before.st_size or mark(os.fstat(fd)) != mark(before)
                or mark(path.lstat()) != mark(before)):
            raise ValueError('unsafe tools')
        return data
    finally:
        os.close(fd)


def trusted_modules():
    release = HERE.parent / 'human/release'
    paths = {
        'board_assembly': HERE / 'assembly.py',
        'release_common': release / 'release_common.py',
        'runtime_checks': release / 'runtime_checks.py',
        'board_r8_ops': release / 'activation_ops.py',
        'board_sync_tools': HERE / 'sync_tools.py',
        'board_candidate': HERE / 'candidate.py',
        'board_binding': HERE / 'binding.py',
        'board_snapshot': HERE / 'snapshot.py',
        'board_ops': HERE / 'activation_ops.py',
        'board_snapshot_worker': release / 'snapshot_worker.py',
    }
    # 全部通过才执行任何 helper；worker 由已有实现另启 -I -B -S 执行。
    source = {name: trusted_source(path) for name, path in paths.items()}
    loaded = {}
    for name, path in paths.items():
        if name == 'board_snapshot_worker':
            continue
        if name in sys.modules:
            raise ValueError('unexpected helper')
        module = types.ModuleType(name)
        module.__file__ = str(path)
        sys.modules[name] = module
        exec(compile(source[name], str(path), 'exec'), module.__dict__)
        loaded[name] = module
    return loaded


def deploy(ops):
    stage, stopped = 'record', False
    try:
        ops.begin()
        stage = 'preflight'
        ops.preflight()
        stage, stopped = 'stop-old', True
        ops.stop()
        stage = 'backup'
        ops.backup()
        stage = 'install-units'
        ops.install()
        stage = 'start-human'
        ops.start_human()
        stage = 'start-agent'
        ops.start_agent()
        stage = 'accept'
        acceptance = ops.accept()
        receipt = {'status': 'accepted', 'build': ops.build,
                   'at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                   'deployment_record': str(ops.record), 'database_rollback': False,
                   'production_credential_invalidation': False, **acceptance}
        ops.receipt('accepted', receipt)
        return 0, receipt
    except BaseException:
        # INT/TERM/HUP 只中断正常流程一次；恢复有超时，第二个信号不再打断。
        # SIGKILL/断电无法保证自动恢复，保留私有记录供操作员核验。
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(signum, signal.SIG_IGN)
        recovery = 'not_needed'
        if stopped:
            try:
                recovery = ops.recover()
            except BaseException:
                recovery = 'blocked_requires_operator'
        receipt = {'status': 'failed', 'build': ops.build, 'stage': stage,
                   'at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                   'deployment_record': str(ops.record) if ops.record else None,
                   'program_recovery': recovery, 'database_rollback': False,
                   'production_credential_invalidation': False}
        if ops.record:
            try:
                ops.receipt('failed', receipt)
            except BaseException:
                receipt['failure_receipt_persisted'] = False
        return 1, receipt
    finally:
        ops.close()


def interrupted(_signum, _frame):
    raise InterruptedError('interrupted')


def options(argv, c):
    parser = c.ClosedParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--manifest-sha256', required=True)
    args = parser.parse_args(argv)
    c.require(len(argv) == 2 and re.fullmatch('[0-9a-f]{64}', args.manifest_sha256))
    return args


def main():
    if not (sys.platform == 'linux' and os.getuid() == os.geteuid() == 0
            and sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode):
        raise ValueError('unsafe invocation')
    os.umask(0o077)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    modules = trusted_modules()
    c = modules['release_common']
    args = options(sys.argv[1:], c)
    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, interrupted)
    ops = modules['board_ops'].Operations(args.manifest_sha256,
        modules['board_assembly'], c, modules['runtime_checks'], modules['board_r8_ops'])
    code, receipt = deploy(ops)
    print(json.dumps(receipt, sort_keys=True))
    return code


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except Exception:
        print(json.dumps({'status': 'rejected_before_record'}))
        raise SystemExit(1)
