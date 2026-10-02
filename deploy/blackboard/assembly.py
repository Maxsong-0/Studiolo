"""从已激活发行派生候选的固定装配；不读取凭据、不操作服务或数据库。

黑板接线沿用 r2，Bridge接线沿用已激活r13；本版仅替换发行路径。
写根、认证、配对和同步范围保持现场原值。
变换只接受钉住的上一发行形状；额外差异由现场摘要和逐字变换核验拒绝。

沿革：r8→r2 的装配逻辑见本文件在 main `40fdf8d` 及更早的版本（一次性迁移，已激活）；
r2→r3（2026-09-22 T-102，已激活）；r3→r4（T-103/T-104，前端并发闸门＋会话每日复核，已激活）；
r4→r5（T-105：服务端认证许可排队＋准确报错，换人侧程序，已激活）；
r5→r6（T-159：T-130 新前端＋main 人侧程序一起换，含只读接口 /api/v1/board/overview，已激活）；
r6→r7（T-167：T-161～T-166 上线后修复，换网页与人侧程序——woff2／svg 长缓存在人侧程序里，已激活）；
r7→r8（T-160f：规则级读权限与「规则」视图；人侧程序须先于 agent 换上，才能解析带 rules_whitelist 的能力集，已激活）；
r8→r9（T-160g：权限矩阵改版，仅换网页，已激活）；
r9→r10（T-132：更新人侧程序，并显式交付 maintain 与 sync-launcher，已激活）；
r10→r11（BUG-020：逐次载荷身份校验，已激活）；
r11→r12（BUG-021：退出观察与停止留证，已激活）；
r12→r13（BUG-022目录初始化、BUG-023保全数量、BUG-024执行器关停、BUG-025陈旧预约取消，已激活）；
本版 r13→r14（BUG-026：同步运行步骤与无载荷失败分类日志）。
"""
import shlex
from pathlib import Path

OLD_RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/m5m6-board-20260920-r13'
BUILD = 'm5m6-board-20260920-r14'
BRIDGE_ROOT = '/var/lib/studiolo-owner/bridge'
TRANSPORT_ROOT = '/home/maxsong/.local/share/studiolo-sync-transport'
SYNC_APPROVAL = '/home/maxsong/.config/studiolo-human/sync-transport.json'
RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/' + BUILD
BASE = '/home/maxsong/.local/share/'
VAULT = BASE + 'studiolo-canonical'
WRITER = BASE + 'studiolo-canonical-writer'
BOARD = BASE + 'studiolo-board'
AGENT = BASE + 'studiolo-agent'
BOARD_RUNTIME = '/run/studiolo-board'
BOARD_SOCKET = BOARD_RUNTIME + '/board.sock'
# 2026-10-01 现场：human r13已接Bridge、agent t160；固定摘要防止覆盖其他发布。
HUMAN_SHA = '0678fd5004b3c2edb08df8acc30eef567dc4a14ef544c818260a25659822b321'
AGENT_SHA = '2e3d9ddfdc1344a448ae1e486afb0314fd94af48ca00f5ce7685a98591db226f'
# 键名沿用早期协议，内容钉住当前 r13 manifest。
R8_MANIFEST_SHA = '1f450940032dc6b97bad554ab72097d76b0229b6f67416d776fc81ba95e829f1'
# 发行目录里属于网页的前缀：本次唯一允许整体更换的部分。
WEB_PREFIX = 'web/'
# 沿用黑板装配标识；Bridge 接线仍受固定现场摘要约束。
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
    for path in (VAULT, WRITER, BOARD, AGENT, BOARD_RUNTIME, BRIDGE_ROOT, TRANSPORT_ROOT):
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
    """已批准Bridge接线逐字沿用，只替换发行路径。"""
    if RELEASE in text or text.count('"--sync-transport"') != 1:
        raise ValueError('unrecognized launcher')
    for fragment in ('"--canonical-writer-data", WRITER', '"--board-socket", BOARD_SOCKET',
                     'BOARD_SOCKET = ' + repr(BOARD_SOCKET), 'VAULT = ' + repr(VAULT),
                     'RELEASE + "/web"'):
        if fragment not in text:
            raise ValueError('board wiring missing')
    if ('正式黑板库 · Obsidian 同步试跑' not in text
            or '"--sync-transport", "' + SYNC_APPROVAL + '",' not in text):
        raise ValueError('bridge wiring missing')
    text = replace_once(text, 'RELEASE = "' + OLD_RELEASE + '"',
                        'RELEASE = "' + RELEASE + '"')
    if OLD_RELEASE in text:
        raise ValueError('stale release path retained')
    compile(text, str(Path(RELEASE) / 'launch-human.py'), 'exec')
    return text
