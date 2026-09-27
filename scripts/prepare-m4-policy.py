#!/usr/bin/env python3
"""把已核验的线上 M4 单元绑定为可检查的策略启用操作包；不联网、不安装、不 bootstrap。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parent.parent
BASE = '/home/maxsong/.local/share/'
POLICY = BASE + 'studiolo-agent-policy'
FILES = ['previous.service', 'policy.service', 'claude.json', 'm4_recovery.py',
         'check-agent.py', 'check-agent-policy.py', 'check-agent-http.py',
         'check-agent-replica.py', 'check-policy-revocation.py']


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def render_unit(raw):
    text = raw.decode('utf-8')
    lines = text.splitlines(keepends=True)
    starts = [i for i, line in enumerate(lines) if line.startswith('ExecStart=')]
    writes = [i for i, line in enumerate(lines) if line.startswith('ReadWritePaths=')]
    if len(starts) != 1 or len(writes) != 1 or '--policy' in text:
        raise ValueError('单元已接入策略或存在多条覆盖指令')
    index = starts[0]
    pattern = (r'ExecStart=(/home/maxsong/\.local/share/studiolo-agent-releases/'
               r'[A-Za-z0-9][A-Za-z0-9._-]{0,79})/bin/studiolo-agent serve '
               r'--replica-root /home/maxsong/\.local/share/studiolo-agent-replica '
               # 白名单随项目增减而变；正则不再钉死具体条目——钉死的话每次扩容都要跟着改这里
               # （T-095 已被迫改过一次）。只校验「至少一个 --allow」，条目内容由部署模板决定。
               r'--replica-max-age-secs 300 (?:--allow (?:"[^"]+"|\S+) )+'
               r'--data /home/maxsong/\.local/share/studiolo-agent '
               r'--index-cache /home/maxsong/\.local/share/studiolo-agent-index --port 4400\n')
    match = re.fullmatch(pattern, lines[index])
    expected_write = 'ReadWritePaths=' + BASE + 'studiolo-agent ' + BASE + 'studiolo-agent-index\n'
    if match is None or lines[writes[0]] != expected_write:
        raise ValueError('单元参数不符合已核验的当前 M4 部署')
    for required in ['User=maxsong', 'Group=maxsong', 'LimitMEMLOCK=infinity',
                     'ProtectSystem=strict', 'ProtectHome=read-only', 'NoNewPrivileges=yes']:
        if lines.count(required + '\n') != 1:
            raise ValueError('缺少唯一的服务身份或隔离前置')
    lines[index] = lines[index].removesuffix('\n') + ' --policy ' + POLICY + '\n'
    lines[writes[0]] = expected_write.removesuffix('\n') + ' ' + POLICY + '\n'
    return ''.join(lines).encode(), match[1]


def prepare(unit_file, binary_sha, output):
    if not re.fullmatch('[0-9a-f]{64}', binary_sha):
        raise ValueError('程序摘要必须是小写 SHA-256')
    output, unit_file = Path(output), Path(unit_file)
    if not output.is_absolute() or output.exists() or output.is_symlink():
        raise ValueError('输出必须是未存在的绝对目录')
    if not unit_file.is_file() or unit_file.is_symlink():
        raise ValueError('单元来源必须是普通文件')
    raw = unit_file.read_bytes()
    unit, release = render_unit(raw)
    sources = {name: ROOT / 'scripts' / name for name in FILES if name.startswith('check-')}
    sources['m4_recovery.py'] = ROOT / 'deploy/agent/m4_recovery.py'
    sources['check-policy-revocation.py'] = ROOT / 'deploy/agent/check-policy-revocation.py'
    material = {'previous.service': raw, 'policy.service': unit,
                'claude.json': (json.dumps({
                    'ai_type': 'claude',
                    # 缺 rules_whitelist＝一条规则都读不到（ADR-026 D4）；首批规则与 docs/m5 初始能力集一致。
                    'read': {'project_whitelist': ['项目/Studiolo', '_规则'], 'deny_by_default': True,
                             'rules_whitelist': ['通用工作规则.md', 'CLAUDE.md']},
                    'write': {'board': False, 'content_patch_only': True, 'content_direct': False},
                    'policy': {'propose_desired': True, 'activate': False},
                    'hard_bound_skills': []}, ensure_ascii=False, indent=2) + '\n').encode()}
    for name, path in sources.items():
        if not path.is_file() or path.is_symlink():
            raise ValueError('缺少完整操作源文件')
        material[name] = path.read_bytes()
    manifest = ''.join(digest(material[name]) + '  ' + name + '\n' for name in FILES).encode()
    template = (ROOT / 'deploy/agent/enable-m4-policy.sh.in').read_text()
    for key, value in {'RELEASE': release, 'BINARY_SHA': binary_sha, 'OLD_UNIT_SHA': digest(raw),
                       'NEW_UNIT_SHA': digest(unit), 'MANIFEST_SHA': digest(manifest)}.items():
        template = template.replace('@' + key + '@', value)
    if re.search(r'@[A-Z_]+@', template):
        raise ValueError('操作模板仍有未绑定项')
    material['FILES.sha256'] = manifest
    material['apply.sh'] = template.encode()
    output.mkdir(mode=0o700)
    # 不自动清理操作包：部分输出也保留供诊断，绝不误删用户的旧目录。
    for name, body in material.items():
        with (output / name).open('xb') as file:
            os.fchmod(file.fileno(), 0o600)
            file.write(body)
    return output / 'apply.sh'


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--unit', required=True)
    parser.add_argument('--binary-sha', required=True)
    parser.add_argument('--output', required=True)
    args = parser.parse_args(argv)
    try:
        script = prepare(args.unit, args.binary_sha, args.output)
    except (OSError, ValueError):
        print('准备失败：请核对线上单元、摘要、完整源码及新的绝对输出目录。', file=sys.stderr)
        return 1
    print(f'已准备：{script}\napply.sh SHA-256：{digest(script.read_bytes())}\n尚未安装或修改任何服务。')
    return 0


if __name__ == '__main__':
    sys.exit(main())
