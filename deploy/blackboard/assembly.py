"""从已激活的上一发行派生下一发行：只换网页，其余逐字节沿用；不读取凭据、不操作服务或数据库。

r2 已经把黑板接线（单元的 RuntimeDirectory／ReadWritePaths、启动器的 board 参数）装好了，
此后的发行**不再做任何装配改造**，只做两件事：把发行路径从上一发行指到本发行，把 `web/` 换成新构建。
因此三个变换函数都是「路径替换 ＋ 现状核验」，多出来的差异一律拒绝。

沿革：r8→r2 的装配逻辑见本文件在 main `40fdf8d` 及更早的版本（一次性迁移，已激活）；
r2→r3（2026-09-22 T-102，已激活）；r3→r4（T-103/T-104，前端并发闸门＋会话每日复核，已激活）；
本版 r4→r5（T-105：服务端认证许可排队＋准确报错，换人侧程序）。
"""
import shlex
from pathlib import Path

OLD_RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/m5m6-board-20260920-r4'
BUILD = 'm5m6-board-20260920-r5'
RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/' + BUILD
BASE = '/home/maxsong/.local/share/'
VAULT = BASE + 'studiolo-canonical'
WRITER = BASE + 'studiolo-canonical-writer'
BOARD = BASE + 'studiolo-board'
AGENT = BASE + 'studiolo-agent'
BOARD_RUNTIME = '/run/studiolo-board'
BOARD_SOCKET = BOARD_RUNTIME + '/board.sock'
# 现场两个单元的摘要：人侧是 r4 激活时装上的那一版；agent 是 T-101 于 2026-09-22 17:36 UTC
# 换装的那一版（读取范围扩到 23 项、程序换成 t101-f234145）。预检以此确认起点没被别人改过。
HUMAN_SHA = '96676afd3bd3f099113cbcca39bfa4851a17dc246cc7c3db870f8c51fa195a7f'
AGENT_SHA = 'f0a3a7166626646dc2ae63a5c0e9a8baf8bdb6a64cf8a1a1de1cffe85bafe763'
# 上一发行（r4）manifest 的摘要。键名沿用 r8 时代的写法，manifest 的线格式因此保持不变。
R8_MANIFEST_SHA = '3ebe6f6499d2ec98c3f774549f728f254a570af208c745437fa2bd73f66667c9'
# 发行目录里属于网页的前缀：本次唯一允许整体更换的部分。
WEB_PREFIX = 'web/'
# 上一发行的装配名；候选包沿用同一装配，只是换了网页。
ASSEMBLY = 'canonical-board-consumer-only'


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('unrecognized source assembly')
    return text.replace(old, new)


def human_unit(text):
    """上一发行的人侧单元已带黑板接线；只把发行路径指到本发行，其余必须逐字不变。"""
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
    """上一发行的启动器已带黑板参数；只把发行路径换成本发行，其余逐字不变，并要求仍能编译。"""
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
