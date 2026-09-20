#!/usr/bin/env python3
"""在Linux私有源码副本中跑虚构快照测试，不更改共享目录权限或携带生产数据。"""
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile

FILES = [
    'deploy/human/offline/human_snapshot.py',
    'deploy/human/offline/human_snapshot_io.py',
    'deploy/human/offline/human_snapshot_format.py',
    'deploy/human/offline/human_snapshot_core.py',
    'deploy/human/offline/test_human_snapshot.py',
    'deploy/human/offline/test_human_snapshot_sources.py',
    'deploy/human/offline/test_human_snapshot_verify.py',
    'deploy/human/offline/test_human_snapshot_fixture.py',
    'deploy/agent/offline/pack_offline.py',
    'deploy/agent/offline/pack_offline_helpers.py',
    'deploy/agent/offline/test_pack_offline.py',
    'deploy/agent/offline/test_pack_human.py',
    'deploy/agent/m4_recovery.py',
    'scripts/replica_common.py', 'scripts/replica_store.py', 'scripts/replica_cleanup.py',
]


def identity(st):
    return (st.st_dev, st.st_ino, st.st_mode, st.st_uid, st.st_gid,
            st.st_nlink, st.st_size, st.st_mtime_ns, st.st_ctime_ns)


def main():
    if sys.platform != 'linux' or len(sys.argv) != 1:
        raise ValueError('仅接受原生Linux无参数测试')
    os.umask(0o077)
    source = Path(__file__).absolute().parents[3]
    with tempfile.TemporaryDirectory(prefix='studiolo-snapshot-tests-') as temporary:
        stage = Path(temporary)
        hashes = {}
        for relative in FILES:
            origin = source / relative
            before = origin.lstat()
            if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1 or before.st_size > 1024**2:
                raise ValueError('虚构测试源码不是有界普通文件')
            raw = origin.read_bytes()
            after = origin.lstat()
            if identity(before) != identity(after) or len(raw) != before.st_size:
                raise ValueError('虚构测试源码发生变化')
            target = stage / relative
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            with target.open('xb') as stream:
                stream.write(raw)
            target.chmod(0o600)
            hashes[relative] = hashlib.sha256(raw).hexdigest()
        print(json.dumps({'fictional_sources_sha256': hashes}, sort_keys=True), flush=True)
        commands = [
            ['-m', 'unittest', 'discover', '-s', 'deploy/human/offline', '-p', 'test_human_snapshot*.py'],
            ['deploy/agent/offline/test_pack_human.py', '--source', str(stage)],
            ['deploy/agent/offline/test_pack_offline.py', '--source', str(stage)],
        ]
        for args in commands:
            result = subprocess.run([sys.executable, '-I', '-B', *args], cwd=stage,
                                    stdin=subprocess.DEVNULL, timeout=180)
            if result.returncode:
                return result.returncode
        # 确认冻结后本地源码仍一致，回执不冒充变化后的工作树验证。
        if any(hashlib.sha256((source / relative).read_bytes()).hexdigest() != digest
               for relative, digest in hashes.items()):
            raise ValueError('测试期间原源码已变化')
    print('全部虚构快照测试通过，私有源码副本已清理。')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
