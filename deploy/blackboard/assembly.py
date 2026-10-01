"""从已激活的上一发行派生下一发行：只换网页，其余逐字节沿用；不读取凭据、不操作服务或数据库。

r2 已经把黑板接线（单元的 RuntimeDirectory／ReadWritePaths、启动器的 board 参数）装好了，
此后的发行**不再做任何装配改造**，只做两件事：把发行路径从上一发行指到本发行，把 `web/` 换成新构建。
因此三个变换函数都是「路径替换 ＋ 现状核验」，多出来的差异一律拒绝。

沿革：r8→r2 的装配逻辑见本文件在 main `40fdf8d` 及更早的版本（一次性迁移，已激活）；
r2→r3（2026-09-22 T-102，已激活）；r3→r4（T-103/T-104，前端并发闸门＋会话每日复核，已激活）；
r4→r5（T-105：服务端认证许可排队＋准确报错，换人侧程序，已激活）；
r5→r6（T-159：T-130 新前端＋main 人侧程序一起换，含只读接口 /api/v1/board/overview，已激活）；
r6→r7（T-167：T-161～T-166 上线后修复，换网页与人侧程序——woff2／svg 长缓存在人侧程序里，已激活）；
r7→r8（T-160f：规则级读权限与「规则」视图；人侧程序须先于 agent 换上，才能解析带 rules_whitelist 的能力集，已激活）；
r8→r9（T-160g：权限矩阵改版，仅换网页，已激活）；
r9→r10（T-132：更新人侧程序，并显式交付 maintain 与 sync-launcher，已激活）；
本版 r10→r11（BUG-020：修复逐次载荷身份校验；Bridge失败现场恢复仍单独执行）。
"""
import shlex
from pathlib import Path

OLD_RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/m5m6-board-20260920-r10'
BUILD = 'm5m6-board-20260920-r11'
RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/' + BUILD
BASE = '/home/maxsong/.local/share/'
VAULT = BASE + 'studiolo-canonical'
WRITER = BASE + 'studiolo-canonical-writer'
BOARD = BASE + 'studiolo-board'
AGENT = BASE + 'studiolo-agent'
BOARD_RUNTIME = '/run/studiolo-board'
BOARD_SOCKET = BOARD_RUNTIME + '/board.sock'
# 2026-10-01 现场：human r10（Bridge已回退）、agent t160；固定摘要防止覆盖其他发布。
HUMAN_SHA = 'd8ce37129782e494675d307caccee8c202308c1ebb7453e6a1055287e231a971'
AGENT_SHA = '2e3d9ddfdc1344a448ae1e486afb0314fd94af48ca00f5ce7685a98591db226f'
# 键名沿用早期协议，内容钉住当前 r10 manifest。
R8_MANIFEST_SHA = '73863a22c779c8e877d051b7608bed9b617316855054fb48c3a3cff8e3557ed6'
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
