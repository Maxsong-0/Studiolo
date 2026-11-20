"""从已激活发行派生候选的固定装配；不读取凭据、不操作服务或数据库。

黑板接线沿用 r2，Bridge 接线与只读身份库参数沿用已激活 r16；本版替换发行路径，并按 T-216 已批准范围接上备份生产者。
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
r13→r14（BUG-026：同步运行步骤与无载荷失败分类日志，已激活）；
r14→r15（BUG-027：只查元数据，保留SQLite进程锁与版本守卫，已激活）；
r15→r16（T-207：BUG-038 项目目录改名、T-181/T-183/T-185/T-211～T-214 等，人侧启动器加 --agent-data；
控制库 schema 3→4，单向，见 README「r15→r16 是单向的」，已激活）；
本版 r16→r17（T-216：启用备份生产者。启动器加 --backup-config；人侧单元加备份 socket 父目录、封存父目录两个写根，
停止超时 90→100 秒。数据库与状态的版本号不变）。
"""
import shlex
from pathlib import Path

OLD_RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/m5m6-board-20260920-r16'
BUILD = 'm5m6-board-20260920-r17'
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
# 备份生产者（T-216）。私有备份配置和主人配置同目录；两个目录由 deploy/backup-worker 的 tmpfiles 片段建，主服务不建。
# 三样都要在激活之前备好：启动器一带上 --backup-config，读不到配置或绑不上 socket 人侧就拒绝启动。
BACKUP_CONFIG = '/home/maxsong/.config/studiolo-human/backup.json'
BACKUP_RUNTIME = '/run/studiolo-backup'
BACKUP_STAGING = '/var/lib/studiolo-backup-staging'
# 关停要等在途的回收（最多 30 秒）和封存（最多一个冻结预算，批准的是 30 秒）做完，所以不短于 90 秒；
# 激活脚本停人侧服务的命令时限是 110 秒（activation_ops.stop），所以要比它短。test_assembly 两头都钉。
STOP_TIMEOUT_OLD = 'TimeoutStopSec=90\n'
STOP_TIMEOUT = 'TimeoutStopSec=100\n'
# 2026-11-20 现场：human r16（已接 Bridge、带 --agent-data）、agent t215-116f5d7；固定摘要防止覆盖其他发布。
# agent 那个是把线上单元里的发布名 t207-c09b065-20261118 两处换成 t215-116f5d7-20261120 之后的摘要
# （换之前是 15b39395…45b9；T-215 部署只换这两处）。
HUMAN_SHA = '1182733562520035376756620584cc2fc38538609a98f1309150cc87e46fa143'
AGENT_SHA = '394ffe6f55823e7ac8cf7cad02fcb2c34cbee0d284afd3c706d13afe54f50117'
# 键名沿用早期协议，内容钉住当前 r16 manifest。
R8_MANIFEST_SHA = '437d75a8a2d67850833fee6b0880f40cc7042391f5a65ce70f0917316a697b4e'
# 发行目录里属于网页的前缀：本次唯一允许整体更换的部分。
WEB_PREFIX = 'web/'
# 沿用黑板装配标识；Bridge 接线仍受固定现场摘要约束。
ASSEMBLY = 'canonical-board-consumer-only'
# agent 单元里必须还在的读取范围（黑板首建时的三个范围）。库里项目目录 2026-10-23 起都带 -Docs（BUG-038）：
# agent 先按新名发布、线上单元换成新名之后本装配才认，没先发 agent 就在这里失败。
# test_assembly 核对它们都在注册表里给 AI 看，下次改名先在测试里报。
AGENT_READ_SCOPES = frozenset({'项目/Studiolo-Docs', '项目/Tillerwright-Docs', '_规则'})


def replace_once(text, old, new):
    if text.count(old) != 1:
        raise ValueError('unrecognized source assembly')
    return text.replace(old, new)


def human_unit(text):
    """上一发行的人侧单元已带黑板与 Bridge 接线；本版指到新发行、加备份的两个写根、放宽停止超时，其余必须逐字不变。"""
    if text.count(OLD_RELEASE) != 2 or RELEASE in text:
        raise ValueError('unrecognized human unit')
    if 'RuntimeDirectory=studiolo-human studiolo-board\n' not in text:
        raise ValueError('board wiring missing')
    for path in (VAULT, WRITER, BOARD, AGENT, BOARD_RUNTIME, BRIDGE_ROOT, TRANSPORT_ROOT):
        if 'ReadWritePaths=' + path + '\n' not in text:
            raise ValueError('board wiring missing')
    # 上一发行还没接备份：出现任何备份路径、或停止超时不是原来那一行，都说明现场不是钉住的形状。
    if 'studiolo-backup' in text or text.count('TimeoutStopSec=') != 1:
        raise ValueError('unexpected backup wiring')
    text = replace_once(text, STOP_TIMEOUT_OLD, STOP_TIMEOUT)
    last_root = 'ReadWritePaths=' + TRANSPORT_ROOT + '\n'
    text = replace_once(text, last_root, last_root + ''.join(
        'ReadWritePaths=' + path + '\n' for path in (BACKUP_RUNTIME, BACKUP_STAGING)))
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
    if not AGENT_READ_SCOPES <= allow_scopes(exec_start):
        raise ValueError('read allowlist changed')
    return text


def launcher(text):
    """沿用黑板、Bridge 与只读身份目录接线，增加已批准的备份配置参数；仅生成候选，不激活。"""
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
    if text.count('"--board-agent-data", AGENT, "--agent-data", AGENT,') != 1 or text.count('"--agent-data"') != 1:
        raise ValueError('agents wiring missing')
    if '--backup-config' in text:
        raise ValueError('unexpected backup wiring')
    approval = '"--sync-transport", "' + SYNC_APPROVAL + '",'
    text = replace_once(text, approval, approval + ' "--backup-config", "' + BACKUP_CONFIG + '",')
    text = replace_once(text, 'RELEASE = "' + OLD_RELEASE + '"',
                        'RELEASE = "' + RELEASE + '"')
    if OLD_RELEASE in text:
        raise ValueError('stale release path retained')
    compile(text, str(Path(RELEASE) / 'launch-human.py'), 'exec')
    return text
