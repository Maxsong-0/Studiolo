"""只捕获和操作明确的虚构配对画面，绝不调用真实候选的main。"""
import json
import os
import select
import subprocess
import time

SOURCE='/home/maxsong/.local/share/studiolo-human-releases/m4-t092-20260920-r4/resume-replacement.py'
FIXTURE='/home/maxsong/t092-r4-test-resume-pty.py'
cases = {
    'success':'completed',
    'reprompt':'completed',
    'interrupt':'T092/screen-confirm/interrupted/0',
    'corrupt':'T092/decrypt/check-failed/0',
    'unenrolled':'T092/state/check-failed/0',
}
for scenario, expected in cases.items():
    command='sudo -n python3 -I -B -S ' + FIXTURE + ' ' + SOURCE + ' ' + scenario
    process=subprocess.Popen(['ssh','-tt','-o','BatchMode=yes','maxsong@192.168.100.169',command],
                             stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,
                             env={**os.environ,'TERM':'xterm-256color'})
    data=b''; first=False; second=False; repeated=False; deadline=time.monotonic()+30
    try:
        while time.monotonic()<deadline:
            if select.select([process.stdout],[],[],0.2)[0]:
                chunk=os.read(process.stdout.fileno(),8192)
                if not chunk:break
                data+=chunk
                if '只按回车开始：'.encode() in data and not first:
                    process.stdin.write(b'\n');process.stdin.flush();first=True
                if '这里不输入验证码：'.encode() in data and not second:
                    value=b'\x03' if scenario=='interrupt' else (b'fictional-input\n' if scenario=='reprompt' else b'\n')
                    process.stdin.write(value);process.stdin.flush();second=True
                if '这里不用输入任何内容'.encode() in data and not repeated:
                    process.stdin.write(b'\n');process.stdin.flush();repeated=True
            if process.poll() is not None:break
        if process.poll() is None:process.terminate()
        exitcode=process.wait(timeout=5)
        marker=b'FIXTURE_RESULT '
        if marker not in data:
            raise RuntimeError('fixture result absent; captured content intentionally suppressed')
        result=json.loads(data.split(marker)[-1].split(b'\r')[0].split(b'\n')[0])
        shown=b'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' in data
        cleared=b'\x1b[2J\x1b[H\x1b[?1049l' in data
        expected_screen=scenario not in ('corrupt','unenrolled')
        passed=exitcode==0 and result['code']==expected and result['state_unchanged'] and shown==expected_screen and cleared==expected_screen
        if scenario=='reprompt':passed=passed and repeated and b'fictional-input' not in data
        print(json.dumps({**result,'screen_seen':shown,'screen_cleared':cleared,'passed':passed}),flush=True)
        if not passed:raise SystemExit(1)
    finally:
        if process.poll() is None:process.kill()
