#!/usr/bin/python3
"""真实PTY里的完整续接合成测试；生产路径只用于加载程序，不访问生产状态。"""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import pwd
import shutil
import sqlite3
import subprocess
import sys
import tempfile

source = Path(sys.argv[1])
scenario = sys.argv[2]
spec = importlib.util.spec_from_file_location('resume_fixture', source)
resume = importlib.util.module_from_spec(spec)
spec.loader.exec_module(resume)
scratch = Path(tempfile.mkdtemp(prefix='t092-resume-fixture-', dir='/var/lib'))
owner = pwd.getpwnam('maxsong')
try:
    database = scratch/'auth.sqlite3'
    with sqlite3.connect(database) as db:
        db.execute('PRAGMA user_version=4')
        db.execute('CREATE TABLE totp_factor(seed_ref TEXT, enrollment_json TEXT)')
        db.execute('CREATE TABLE totp_pending(id INTEGER)')
        db.execute('CREATE TABLE recovery_pending(id INTEGER)')
        if scenario != 'unenrolled':
            db.execute("INSERT INTO totp_factor VALUES('studiolo-human-totp-v1', '{}')")
    config = scratch/'owner.json'
    config.write_text(json.dumps({'seed_ref':'studiolo-human-totp-v1', 'auth_directory':str(scratch)}))
    for path in (database, config):
        os.chown(path, owner.pw_uid, owner.pw_gid)
        path.chmod(0o600)
    candidate = scratch/'fictional.cred'
    result = subprocess.run(['/usr/bin/systemd-creds','encrypt','--name=totp-next','--with-key=host','-','-'],
                            input=b'00'*20, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert result.returncode == 0
    candidate.write_bytes(result.stdout if scenario != 'corrupt' else b'fictional-invalid-cipher')
    candidate.chmod(0o600)
    observed = (candidate, database, config)
    before = [hashlib.sha256(path.read_bytes()).digest() for path in observed]
    resume.CONFIG, resume.NEXT, resume.RELEASE = config, candidate, source.parent
    sys.argv = [str(source)]
    code = 'completed'
    try:
        resume.main()
    except (Exception, KeyboardInterrupt) as error:
        code = resume.failure_code(error)
    unchanged = before == [hashlib.sha256(path.read_bytes()).digest() for path in observed]
    print('FIXTURE_RESULT ' + json.dumps({'scenario':scenario,'code':code,'state_unchanged':unchanged}), flush=True)
finally:
    shutil.rmtree(scratch)
