"""从已激活发行派生候选的固定装配；不读取凭据、不操作服务或数据库。

黑板接线沿用 r2，Bridge 接线与只读身份库参数沿用已激活 r16；本版替换发行路径，并按 T-216 已批准范围接上备份生产者。
已有写根、认证、配对和同步范围保持现场原值，本版另加备份的两个写根。
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
停止超时 90→195 秒；人侧程序必须一起换，上一发行的程序不认 --backup-config。数据库与状态的版本号不变）。
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
# 激活预检在停服之前核这些前置（activation_ops.backup_ready），缺了就不停服务。
BACKUP_CONFIG = '/home/maxsong/.config/studiolo-human/backup.json'
BACKUP_RUNTIME = '/run/studiolo-backup'
BACKUP_STAGING = '/var/lib/studiolo-backup-staging'
# 本版启动器新带的参数：上一发行的人侧程序不认它，沿用旧程序的候选必然起不来。
# 非空就表示本版必须换人侧程序：出候选和候选核验（激活预检也跑它）都要求候选程序的字节和上一发行不同，
# 并且字节里有这些字面量（参数名在程序里是原样的字符串，crates/local-server/src/options/table.rs；
# 169 上 r16 的程序里没有 --backup-config，2026-11-20 只读核过）。只看来源提交号不够：号可以填新的、文件却是旧的。
NEW_HUMAN_PROGRAM_LITERALS = (b'--backup-config',)
REQUIRES_NEW_HUMAN_PROGRAM = bool(NEW_HUMAN_PROGRAM_LITERALS)
# 人侧停止超时（秒）。关停是串行的两段（crates/local-server/src/startup/serving.rs）：收到退出信号先等备份收尾，
# 做完才通知 Bridge 停，所以两段要相加。备份段等的是在途的那一个连接整个处理完，再等它起的后台线程：
#   收发窗口：socket 收发的期限（service.rs 的 WINDOW）；
#   处理请求：不受收发窗口管，要拿状态锁读账本，每次读按 SCAN_BUDGET 算。
#             自己最多读三次：安排封存（producer.rs 的 schedule 两次加它调的 decide 一次）和安排抽查
#             （producer/check.rs 的 repository_check_with）都是三次，别的请求更少；
#             等锁时别人读两次：别的会拿着这把锁读账本的只有健康页（worker 的请求在同一个接受循环里一个个处理，
#             service.rs；后台线程只在收尾时拿一下），一次健康页快照先后拿两次锁、各读一次
#             （startup/health_input.rs：备份作业、仓库抽查）；
#   一次回收（RECLAIM_BUDGET）加一次封存（冻结和导出在一个冻结预算之内；这里按批准的 30 秒算）。
# Bridge 段 = 同步审批的 stop_grace_seconds（169 现行 30）+ 60（deploy/sync-bridge/README.md 的要求）。
# 最后一项是余量，只给不受预算管的小步骤：建暂存区、写准入或失败结果。
#
# 这个数是各项按上限相加的预算，不是保证；程序里也没有一个关停总期限兜着。下面这些没有算进去：
#   - 读账本是读完一页才看预算，单次可能多出一页的时间；
#   - 后台线程收尾时要拿一下状态锁把 busy 复位，关停时会跟着等健康页的那次读完；同时在途的健康页快照也可能不止一个；
#   - 接受循环没有规定先看停止信号：信号到达时正好排着的连接可能还会被处理。每多处理一个，按上面的算法就多一个
#     收发窗口加几次读账本（35 秒）；后台空闲时它还会再起一次回收，到期的话再加一次封存
#     （程序里另案修，见 docs/contracts/backup-producer.md「已知限制」）。
# 余量吸收不了这些。能兜住的是各项很少同时顶到上限：读账本的各处共用一个读位置，别人读掉的积压请求就不用再读，
# 请求自己的第二、三次读只读上一次读完之后新追加的（一般就是它自己刚写的几条），所以实际用时通常远小于这个数。
#
# 超时不是无害的。systemd 到点强杀时关停链还没走完：人侧 HTTP 入口（/run/studiolo-human/http.sock）和黑板入口
# （BOARD_SOCKET）要到最后、排空 HTTP 和停黑板之后才删（serving.rs 的 drain、unix_ingress.rs），单元又是
# RuntimeDirectoryPreserve=yes，两个节点都会留下。留下之后人侧下次启动按设计拒绝，激活脚本停服后的核对
# （activation_ops.Operations.stopped）也拒绝，激活和回滚都停下来等人看过再删。备份自己这边：socket 在接受循环结束时
# 就删了；封存做到一半被杀，留下的原意图下一次 schedule 补失败结果，暂存一般由下次启动的第一轮回收收走，但正好在
# 「建目录」和「登记落盘」之间被杀的话回收会失败关闭、之后的封存全部失败，也要人处理。
# 所以这个数宁大勿小；调它之前先看上面没算进去的那几样。
#
# test_assembly 对着源码钉：程序里的三个常量、生产者源码里读账本和拿状态锁的每一处、请求怎么分派、健康页快照调哪两个探针
# （钉的是源码文本，认不出的写法见那条用例的说明）。
# 冻结预算和停止宽限来自 backup.json 和同步审批，激活预检核对现场没有配得更大。
# 停服务的命令时限在激活脚本那头按这个值再加余量算（activation_ops.STOP_COMMAND_MARGIN_SECONDS），不另写一个数。
BACKUP_CONNECTION_WINDOW_SECONDS = 10
BACKUP_SCAN_BUDGET_SECONDS = 5
BACKUP_REQUEST_OWN_SCANS = 3
BACKUP_SNAPSHOT_SCANS = 2
BACKUP_REQUEST_SCANS = BACKUP_REQUEST_OWN_SCANS + BACKUP_SNAPSHOT_SCANS
BACKUP_RECLAIM_SECONDS = 30
BACKUP_FREEZE_BUDGET_SECONDS = 30
BRIDGE_STOP_GRACE_SECONDS = 30
BRIDGE_STOP_MARGIN_SECONDS = 60
UNBUDGETED_STEPS_SECONDS = 10
OLD_STOP_TIMEOUT_SECONDS = 90
# 10 + 5×(3+2) + 30 + 30 + 30 + 60 + 10 = 195；test_assembly 钉这个结果，任一项改了要一起看。
STOP_TIMEOUT_SECONDS = (BACKUP_CONNECTION_WINDOW_SECONDS + BACKUP_SCAN_BUDGET_SECONDS * BACKUP_REQUEST_SCANS
                        + BACKUP_RECLAIM_SECONDS + BACKUP_FREEZE_BUDGET_SECONDS
                        + BRIDGE_STOP_GRACE_SECONDS + BRIDGE_STOP_MARGIN_SECONDS + UNBUDGETED_STEPS_SECONDS)
# 固定摘要防止覆盖其他发布。human：2026-11-20 回读的现场值（r16 激活写下的单元，已接 Bridge、带 --agent-data）。
# agent：预算值，不是回读来的。写这一版时线上 agent 还是 t207-c09b065-20261118（单元摘要 15b39395…45b9）；
# T-215 部署只把单元里的发布名两处换成 t215-116f5d7-20261120，这里钉的是换完之后应有的摘要。
# agent 换完要回读一致；不一致就改这里重出候选（出候选和激活预检都会因摘要不符而拒绝，不会带着错值上线）。
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


def stop_timeout_line(seconds):
    """单元里停止超时那一整行。"""
    return 'TimeoutStopSec=%d\n' % seconds


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
    text = replace_once(text, stop_timeout_line(OLD_STOP_TIMEOUT_SECONDS), stop_timeout_line(STOP_TIMEOUT_SECONDS))
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
