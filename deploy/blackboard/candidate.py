"""逐字节验证已部署包和候选包；可选 Bridge 程序遵循独立来源和完整双件套。"""
from pathlib import Path
import re
from board_sync_tools import TOOLS, validate_metadata, validate_transition

NAMES = ('studiolo-human', 'studiolo-agent')

# manifest 的闭集：r2 那一版没有 web_source_commit，r3 起必须有。
MANIFEST_KEYS = {'build', 'bootstrap_source_commit', 'human_program_source_commit',
                 'web_source_commit', 'r8_manifest_sha256', 'assembly', 'before_units', 'files'}
OPTIONAL_KEYS = {'sync_program_source_commit'}


def verify_previous(a, c):
    """上一发行也是黑板形状的 manifest，用同一闭集逐项核验；不沿用 r8 时代的 verifier。"""
    root = Path(a.OLD_RELEASE)
    raw = c.read(root / 'manifest.json', c.UID, 0o600)
    c.require(c.sha(raw) == a.R8_MANIFEST_SHA)
    manifest = c.document(raw)
    c.require(MANIFEST_KEYS - {'web_source_commit'} <= set(manifest) <= MANIFEST_KEYS | OPTIONAL_KEYS)
    validate_metadata(manifest, c)
    c.require(manifest['assembly'] == a.ASSEMBLY and isinstance(manifest['build'], str))
    for key in ('bootstrap_source_commit', 'human_program_source_commit'):
        c.require(isinstance(manifest[key], str) and re.fullmatch('[0-9a-f]{40}', manifest[key]))
    files = manifest['files']
    c.require(isinstance(files, dict) and files)
    c.require(any(name.startswith(a.WEB_PREFIX) for name in files))
    directories = set()
    for name, item in files.items():
        path = c.relative(name)
        c.require(name != 'manifest.json')
        c.require(isinstance(item, dict) and set(item) == {'bytes', 'sha256'})
        c.require(type(item['bytes']) is int and 0 <= item['bytes'] <= c.MAX_FILE)
        c.require(isinstance(item['sha256'], str) and re.fullmatch('[0-9a-f]{64}', item['sha256']))
        directories.update(p.as_posix() for p in path.parents if str(p) != '.')
    actual, dirs = c.scan(root, c.UID)
    c.require(dirs == directories and set(actual) == set(files) | {'manifest.json'})
    c.require(all(actual[name] == item for name, item in files.items()))
    c.require(c.read(root / 'manifest.json', c.UID, 0o600) == raw)
    return manifest


def verify_candidate(a, c, expected_sha):
    """候选包＝上一发行换掉整个 web/（可选换人侧程序）；其余逐字节相同，程序来源不得冒称新构建。"""
    c.require(isinstance(expected_sha, str) and re.fullmatch('[0-9a-f]{64}', expected_sha))
    root = Path(a.RELEASE)
    old = verify_previous(a, c)
    raw = c.read(root / 'manifest.json', c.UID, 0o600)
    c.require(c.sha(raw) == expected_sha)
    manifest = c.document(raw)
    c.require(MANIFEST_KEYS <= set(manifest) <= MANIFEST_KEYS | OPTIONAL_KEYS)
    tool_changes = validate_transition(old, manifest, c)
    c.require(manifest['build'] == a.BUILD and manifest['assembly'] == a.ASSEMBLY)
    c.require(re.fullmatch('[0-9a-f]{40}', manifest['bootstrap_source_commit']))
    c.require(isinstance(manifest['human_program_source_commit'], str)
              and re.fullmatch('[0-9a-f]{40}', manifest['human_program_source_commit']))
    # 网页必须能追到构建它的提交；程序沿用上一发行，照抄其来源，不冒称新构建。
    c.require(isinstance(manifest['web_source_commit'], str)
              and re.fullmatch('[0-9a-f]{40}', manifest['web_source_commit']))
    c.require(manifest['bootstrap_source_commit'] == old['bootstrap_source_commit'])
    # 人侧程序：来源提交没变就必须逐字节沿用；来源变了才允许换（仍须是 ELF，见下）。
    program_changed = manifest['human_program_source_commit'] != old['human_program_source_commit']
    c.require(manifest['r8_manifest_sha256'] == a.R8_MANIFEST_SHA)
    c.require(manifest['before_units'] == dict(zip(NAMES, (a.HUMAN_SHA, a.AGENT_SHA))))
    files = manifest['files']
    c.require(isinstance(files, dict))
    # 这三份按现场重新派生：启动器与人侧单元改发行路径，agent 单元照抄现场
    # （读取范围可能在两次发行之间被 L3 改过，必须以现场为准而不是上一发行的存档）。
    changed = {'launch-human.py', 'studiolo-human.service', 'studiolo-agent.service'}
    if program_changed:
        changed.add('bin/studiolo-local-server')
    changed.update(tool_changes)
    old_web = {name for name in old['files'] if name.startswith(a.WEB_PREFIX)}
    new_web = {name for name in files if name.startswith(a.WEB_PREFIX)}
    c.require(new_web and a.WEB_PREFIX + 'index.html' in new_web)
    new_tools = TOOLS if 'sync_program_source_commit' in manifest else set()
    c.require(set(files) == (set(old['files']) - old_web) | new_web | new_tools)
    directories = set()
    for name, item in files.items():
        path = c.relative(name)
        c.require(isinstance(item, dict) and set(item) == {'bytes', 'sha256'})
        c.require(type(item['bytes']) is int and 0 <= item['bytes'] <= c.MAX_FILE)
        c.require(isinstance(item['sha256'], str) and re.fullmatch('[0-9a-f]{64}', item['sha256']))
        if name not in changed and name not in new_web:
            c.require(item == old['files'][name])
        directories.update(p.as_posix() for p in path.parents if str(p) != '.')
    c.inspect(root, c.UID, 0o700, True)
    actual, dirs = c.scan(root, c.UID)
    c.require(dirs == directories and set(actual) == set(files) | {'manifest.json'})
    c.require(all(actual[name] == item for name, item in files.items()))
    for directory in dirs:
        c.inspect(root / directory, c.UID, 0o700, True)
    for name in actual:
        c.inspect(root / name, c.UID, 0o755 if name in c.BINARIES | {'bin/studiolo-vault-admin', 'bin/studiolo-maintain'} else 0o600)
    for name in TOOLS & set(files):
        c.require(c.read(root / name, c.UID, 0o755).startswith(b'\x7fELF'))
    c.require(c.read(root / 'bin/studiolo-vault-admin', c.UID, 0o755).startswith(b'\x7fELF'))
    c.require(c.read(root / 'bin/studiolo-local-server', c.UID, 0o755).startswith(b'\x7fELF'))
    c.require(c.read(root / 'manifest.json', c.UID, 0o600) == raw)
    return manifest


