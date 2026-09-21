"""从已核验的 r8 配置派生黑板接线；不读取凭据、不操作服务或数据库。"""
from pathlib import Path

OLD_RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/m4-t094-20260920-r8'
BUILD = 'm5m6-board-20260920-r1'
RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/' + BUILD
BASE = '/home/maxsong/.local/share/'
VAULT = BASE + 'studiolo-canonical'
WRITER = BASE + 'studiolo-canonical-writer'
BOARD = BASE + 'studiolo-board'
AGENT = BASE + 'studiolo-agent'
BOARD_RUNTIME = '/run/studiolo-board'
BOARD_SOCKET = BOARD_RUNTIME + '/board.sock'
HUMAN_SHA = 'd6e95f78f99d033cb78b8db7017f12ee5b1a12082f3bd689d94edc270bcc1ad1'
AGENT_SHA = '80a9fc7fea7cca94bc4f79f893ca038d9b3470c0aca463390d4d81ed76e94991'
R8_MANIFEST_SHA = '26b2e1fadad9f32a4da64b615b29b71d411e51bd1d4969274b5bc02dd4c0f8d4'


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('unrecognized source assembly')
    return text.replace(old, new)


def human_unit(text):
    if text.count(OLD_RELEASE) != 2 or '--board-' in text:
        raise ValueError('unrecognized human unit')
    text = text.replace(OLD_RELEASE, RELEASE)
    text = replace_once(text, 'RuntimeDirectory=studiolo-human\n',
                        'RuntimeDirectory=studiolo-human studiolo-board\n')
    return replace_once(text, 'ReadWritePaths=/run/studiolo-human\n',
                        'ReadWritePaths=/run/studiolo-human\n' + ''.join(
                            'ReadWritePaths=' + path + '\n'
                            for path in (VAULT, WRITER, BOARD, AGENT, BOARD_RUNTIME)))


def agent_unit(text):
    lines = text.splitlines(keepends=True)
    indexes = [i for i, line in enumerate(lines) if line.startswith('ExecStart=')]
    if len(indexes) != 1 or '--board-socket' in text:
        raise ValueError('unrecognized agent unit')
    i = indexes[0]
    if not all(('--allow ' + scope + ' ') in lines[i]
               for scope in ('项目/Studiolo', '项目/Tillerwright', '_规则')):
        raise ValueError('read allowlist changed')
    lines[i] = lines[i].rstrip('\n') + ' --board-socket ' + BOARD_SOCKET + '\n'
    return ''.join(lines)


def launcher(text):
    text = replace_once(text, 'RELEASE = "' + OLD_RELEASE + '"',
                        'RELEASE = "' + RELEASE + '"')
    text = replace_once(text, 'SOCKET = "/run/studiolo-human/http.sock"',
                        'SOCKET = "/run/studiolo-human/http.sock"\n'
                        + '\n'.join(name + ' = ' + repr(value) for name, value in (
                            ('VAULT', VAULT), ('WRITER', WRITER), ('BOARD', BOARD),
                            ('AGENT', AGENT), ('BOARD_SOCKET', BOARD_SOCKET))))
    text = replace_once(text, 'for directory in (RELEASE + "/example-vault", RELEASE + "/web"):',
                        'for directory in (VAULT, WRITER, BOARD, RELEASE + "/web"):')
    text = replace_once(text, '    parent_fd = open_directory("/run/studiolo-human", private=True)',
                        '    inspect_existing(AGENT + "/agent.sqlite3", 0o600)\n'
                        '    for directory in (VAULT, WRITER, BOARD):\n'
                        '        os.close(open_directory(directory, private=True))\n'
                        '    inspect_existing(VAULT + "/.workbench/broker-binding.json", 0o600)\n'
                        '    inspect_existing(WRITER + "/broker-binding.json", 0o600)\n'
                        '    os.close(open_directory("' + BOARD_RUNTIME + '", private=True))\n'
                        '    require(not os.path.lexists(BOARD_SOCKET))\n'
                        '    parent_fd = open_directory("/run/studiolo-human", private=True)')
    text = replace_once(text, 'BINARY, "--vault", RELEASE + "/example-vault",',
                        'BINARY, "--vault", VAULT,')
    text = replace_once(text, '"--label", "服务器示例库",',
                        '"--label", "正式黑板库 · 初始快照（持续同步未接通）",')
    text = replace_once(text, '"--factor-socket", SOCKET, "--policy-data", POLICY,',
                        '"--factor-socket", SOCKET, "--policy-data", POLICY,\n'
                        '        "--canonical-writer-data", WRITER, "--board-data", BOARD,\n'
                        '        "--board-agent-data", AGENT, "--board-socket", BOARD_SOCKET,')
    compile(text, str(Path(RELEASE) / 'launch-human.py'), 'exec')
    return text
