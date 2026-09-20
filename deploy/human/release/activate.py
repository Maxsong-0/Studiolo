#!/usr/bin/python3
"""固定r7→r8人侧程序发布；不执行任何认证撤权或数据库回滚。"""
import datetime
import json
import os
from pathlib import Path
import re
import resource
import signal
import stat
import sys

HERE = Path(os.path.abspath(__file__)).parent


def trusted_modules():
    for node in [HERE, *HERE.parents]:
        s = node.lstat()
        if not stat.S_ISDIR(s.st_mode) or s.st_uid not in (0, 1000) or s.st_mode & 0o7022:
            raise SystemExit('activate: unsafe tool path')
    for name in ('release_common.py', 'runtime_checks.py', 'activation_ops.py', 'snapshot_worker.py'):
        s = (HERE / name).lstat()
        if not stat.S_ISREG(s.st_mode) or s.st_uid not in (0, 1000) or s.st_nlink != 1 or s.st_mode & 0o7022:
            raise SystemExit('activate: unsafe tool')
    sys.path.insert(0, str(HERE))


def deploy(ops):
    stage, stop_requested, unit_may_have_changed = 'record', False, False
    try:
        ops.begin()
        stage = 'preflight'
        ops.preflight()
        stage, stop_requested = 'stop-old', True
        ops.stop()
        stage = 'backup'
        ops.backup()
        stage, unit_may_have_changed = 'install-unit', True
        ops.install()
        stage = 'start-r8'
        ops.start()
        stage = 'accept-r8'
        acceptance = ops.accept()
        receipt = {'status': 'accepted', 'build': 'm4-t094-20260920-r8',
                   'at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                   'deployment_record': str(ops.record), 'database_rollback': False,
                   'production_credential_invalidation': False, **acceptance}
        ops.receipt('accepted', receipt)
        return 0, receipt
    except BaseException as error:
        # 捕获TERM/INT/HUP后保护有界回退，第二个信号不得打断恢复旧程序。
        for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
            signal.signal(signum, signal.SIG_IGN)
        recovery = 'not_needed'
        if stop_requested:
            try:
                recovery = ops.recover(unit_may_have_changed)
            except BaseException:
                recovery = 'blocked_requires_operator'
        receipt = {'status': 'failed', 'stage': stage, 'error_class': type(error).__name__,
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


def options(argv):
    import release_common as c
    parser = c.ClosedParser(description=__doc__, allow_abbrev=False)
    parser.add_argument('--manifest-sha256', required=True)
    args = parser.parse_args(argv)
    c.require(len(argv) == 2 and re.fullmatch('[0-9a-f]{64}', args.manifest_sha256))
    return args


def main():
    trusted_modules()
    import release_common as c
    from activation_ops import Operations
    args = options(sys.argv[1:])
    c.require(sys.platform == 'linux' and os.getuid() == os.geteuid() == 0)
    c.require(sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode)
    os.umask(0o077)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, interrupted)
    code, receipt = deploy(Operations(args.manifest_sha256))
    print(json.dumps(receipt, sort_keys=True))
    return code


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except Exception:
        print(json.dumps({'status': 'rejected_before_record'}))
        raise SystemExit(1)
