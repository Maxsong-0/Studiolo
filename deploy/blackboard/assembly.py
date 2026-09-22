"""从已激活的 r2 派生 r3：只换网页，其余逐字节沿用；不读取凭据、不操作服务或数据库。

r2 已经把黑板接线（单元的 RuntimeDirectory／ReadWritePaths、启动器的 board 参数）装好了，
本次发行**不再做任何装配改造**，只做两件事：把发行路径从 r2 指到 r3，把 `web/` 换成新构建。
因此三个变换函数都是「路径替换 ＋ 现状核验」，多出来的差异一律拒绝。

沿革：r8→r2 的装配逻辑见本文件在 main `40fdf8d` 及更早的版本（一次性迁移，已激活）。
"""
import shlex
from pathlib import Path

OLD_RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/m5m6-board-20260920-r2'
BUILD = 'm5m6-board-20260920-r3'
RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/' + BUILD
BASE = '/home/maxsong/.local/share/'
VAULT = BASE + 'studiolo-canonical'
WRITER = BASE + 'studiolo-canonical-writer'
BOARD = BASE + 'studiolo-board'
AGENT = BASE + 'studiolo-agent'
BOARD_RUNTIME = '/run/studiolo-board'
BOARD_SOCKET = BOARD_RUNTIME + '/board.sock'
# 现场两个单元的摘要（r2 激活时装上的那一版）：预检以此确认起点就是 r2，而不是别的发行。
HUMAN_SHA = 'd3125e15be711a86dbe9a577af48e1b83b0cf440e8aaebfe43a1505ce4c38d4f'
AGENT_SHA = '3179387c08a03a3f2f22bc2a08009c621453b96e284cbb18e3559f96837aa935'
# 上一发行（r2）manifest 的摘要。键名沿用 r8 时代的写法，manifest 的线格式因此保持不变。
R8_MANIFEST_SHA = '8bf7b6ea412fceebc89d8d5368810db66198ac321573d27b766d894f8aebb353'
# 发行目录里属于网页的前缀：本次唯一允许整体更换的部分。
WEB_PREFIX = 'web/'
# 上一发行的装配名；候选包沿用同一装配，只是换了网页。
ASSEMBLY = 'canonical-board-consumer-only'


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('unrecognized source assembly')
    return text.replace(old, new)


def human_unit(text):
    """r2 的人侧单元已带黑板接线；只把发行路径指到 r3，其余必须逐字不变。"""
    if text.count(OLD_RELEASE) != 2 or RELEASE in text:
        raise ValueError('unrecognized human unit')
    if 'RuntimeDirectory=studiolo-human studiolo-board\n' not in text:
        raise ValueError('board wiring missing')
    for path in (VAULT, WRITER, BOARD, AGENT, BOARD_RUNTIME):
        if 'ReadWritePaths=' + path + '\n' not in text:
            raise ValueError('board wiring missing')
    return text.replace(OLD_RELEASE, RELEASE)


def allow_scopes(exec_start):
    """按 shell 词法取 --allow 的实参：单元里这些范围有时带引号，不能按裸字符串匹配。"""
    tokens = shlex.split(exec_start.split('=', 1)[1] if exec_start.startswith('ExecStart=') else exec_start)
    return {value for token, value in zip(tokens, tokens[1:]) if token == '--allow'}


def agent_unit(text):
    """agent 单元不指向人侧发行目录，本次原样沿用；仍核验黑板 socket 与三个读取范围没被改。"""
    lines = text.splitlines(keepends=True)
    indexes = [i for i, line in enumerate(lines) if line.startswith('ExecStart=')]
    if len(indexes) != 1 or OLD_RELEASE in text or RELEASE in text:
        raise ValueError('unrecognized agent unit')
    exec_start = lines[indexes[0]]
    if ('--board-socket ' + BOARD_SOCKET) not in exec_start:
        raise ValueError('board socket missing')
    if not {'项目/Studiolo', '项目/Tillerwright', '_规则'} <= allow_scopes(exec_start):
        raise ValueError('read allowlist changed')
    return text


def launcher(text):
    """r2 的启动器已带黑板参数；只把发行路径换成 r3，其余逐字不变，并要求仍能编译。"""
    if RELEASE in text:
        raise ValueError('unrecognized launcher')
    for fragment in ('"--canonical-writer-data", WRITER', '"--board-socket", BOARD_SOCKET',
                     'BOARD_SOCKET = ' + repr(BOARD_SOCKET), 'VAULT = ' + repr(VAULT),
                     'RELEASE + "/web"'):
        if fragment not in text:
            raise ValueError('board wiring missing')
    text = replace_once(text, 'RELEASE = "' + OLD_RELEASE + '"',
                        'RELEASE = "' + RELEASE + '"')
    if OLD_RELEASE in text:
        raise ValueError('stale release path retained')
    compile(text, str(Path(RELEASE) / 'launch-human.py'), 'exec')
    return text
