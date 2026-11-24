"""从已激活发行派生候选的固定装配；不读取凭据、不操作服务或数据库。

黑板接线沿用 r2，Bridge 接线与只读身份库参数沿用 r16，备份生产者接线（启动器参数、两个写根、停止超时）沿用 r17
（r18～r23 原样沿用，已激活）；本版替换发行路径、人侧程序和人侧单元的内存上限，写根、认证、配对和同步范围保持现场原值。
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
r16→r17（T-216：启用备份生产者。启动器加 --backup-config；人侧单元加备份 socket 父目录、封存父目录两个写根，
停止超时 90→300 秒；人侧程序必须一起换，上一发行的程序不认 --backup-config。数据库与状态的版本号不变，已激活）；
r17→r18（T-219：备份首跑三处缺陷的修复，BUG-042 队列自己追账本、BUG-044 封存前补记登录库，
加 BUG-045 封存的冻结窗口只定格、拷贝和校验挪到放开之后；只换人侧程序，启动器和单元的接线一个字不动。起点已带备份监听：激活预检核的是活的备份 socket，停服之后核它已经删掉。
数据库与状态的版本号不变，已激活）；
r18→r19（T-222：BUG-046 主库没开文件操作时，Bridge 碰到主库删除整批暂停，不再记成终态未知；
只换人侧程序，启动器和单元的接线一个字不动。数据库与状态的版本号不变：Bridge 库不加表、不改编码，已激活）；
r19→r20（T-226：BUG-047 同步一轮能比对的路径数 4096→16384，健康页加路径与比对内容用量预警；
合版带上 T-224 的同步状态回执 sync-status.json。只换人侧程序，启动器和单元的接线一个字不动。
Bridge 库不加表、不改编码，但新程序一启动就把库里存的路径预算从 4096 升到 16384，r19 打开这样的库会判预算超限、
人侧起不来：对 Bridge 库是单向的，见 README「r19→r20 对 Bridge 库是单向的」，已激活）；
r20→r21（T-227：BUG-050 看板租约巡检遇备份冻结跳过这一轮，不再带着人侧退出；关停链打出谁先结束和起因，
封存打出冻结窗口里和放开之后的分步用时。只换人侧程序，启动器和单元的接线一个字不动。
数据库与状态的版本号不变：Bridge 库不加表、不改编码，路径预算 r20 已升过，激活失败可以退回 r20，已激活）；
r21→r22（T-227：BUG-052 封存的导出扫描也认批次终态记录，2026-11-13 以前的批次提交留下的历史正文
不再让整批封存失败。只换人侧程序，启动器和单元的接线一个字不动。数据库与状态的版本号不变，激活失败可以退回 r21，已激活）；
r22→r23（T-227：BUG-053 worker 读回整份封存只起一个 restic dump，tar 流边读边按清单核。修复在 worker 程序里，
人侧程序和 r22 一字不差，不要求换（NEW_HUMAN_PROGRAM_LITERALS 为空）；要出发行、激活，是因为私有备份配置里 worker 的摘要
和部署版本变了，人侧只在启动时读它。启动器和单元的接线一个字不动。数据库与状态的版本号不变，激活失败可以退回 r22，已激活）；
本版 r23→r24（T-227：BUG-050 封存冻结窗口里不再逐条核写者回执，挪到放开之后、在导出的写者副本上核；BUG-054 封存拷 SQLite 库
每 64 MiB 落一次盘，人侧单元内存上限 512M→2G；BUG-055 封存开头撞上进程内短暂持有的锁不再整轮失败。换人侧程序；
单元除了发行路径只改内存上限那一行，启动器一个字不动。数据库与状态的版本号不变，激活失败可以退回 r23——单元也随之退回 512M，
见 README「r23→r24」一段）。
"""
import shlex
from pathlib import Path

OLD_RELEASE = '/home/maxsong/.local/share/studiolo-human-releases/m5m6-board-20260920-r23'
BUILD = 'm5m6-board-20260920-r24'
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
# 备份监听的 socket：置备脚本写进 backup.json 的就是这一个（deploy/backup-worker/provision/prereq.py 的 SOCKET）。
# 起点（r17 起每一版）已经在监听它：激活预检核它是活的，停服之后核它已经删掉（activation_ops）。
BACKUP_SOCKET = BACKUP_RUNTIME + '/producer.sock'
# 本版必须换的人侧程序里才有的字节：非空就表示本版必须换人侧程序。出候选和候选核验（激活预检也跑它）都要求候选程序的
# 字节和上一发行不同，并且字节里有这些字面量。只看来源提交号不够：号可以填新的、文件却是旧的。
# 本版（r24）的启动器没有新参数，要换程序是因为修复在程序里（封存的冻结窗口和放开之后）。取三段只有新程序才有的字节
# （中文，按 UTF-8 编码后比）：冻结窗口里改名的两步、放开之后核写者回执没过的那一句，都是完整的字符串字面量（最后一句
# 到占位符为止）。编进程序后都是原样的字节。
# 169 上 r23 的程序（38bf34a 构建，摘要 85a941f8…001a）里三段都没有，2026-11-24 只读核过（按字节数，各 0 处）。
NEW_HUMAN_PROGRAM_LITERALS = tuple(literal.encode() for literal in (
    '写者冻结',
    '定格写者两棵树',
    '核写者回执：',
))
REQUIRES_NEW_HUMAN_PROGRAM = bool(NEW_HUMAN_PROGRAM_LITERALS)
# 人侧停止超时（秒）。关停是串行的两段（crates/local-server/src/startup/serving.rs）：收到退出信号先等备份收尾，
# 做完才通知 Bridge 停，所以两段要相加。备份段等的是在途的那一个连接整个处理完，再等它起的后台线程
# （只等一个，靠的是接受循环先看停止信号：信号一到，排着的连接不再接、到点的回收不再起。
#  service.rs 的 accept_until_stopped，同目录 service_tests.rs 钉着）：
#   收发窗口：socket 收发的期限（service.rs 的 WINDOW）；
#   处理请求：不受收发窗口管，要拿状态锁读账本，每次读按 SCAN_BUDGET 算。
#             自己最多读三次：安排封存（producer.rs 的 schedule 两次加它调的 decide 一次）和安排抽查
#             （producer/check.rs 的 repository_check_with）都是三次，别的请求更少；
#             等锁时别人读两次：别的会拿着这把锁读账本的只有健康页（worker 的请求在同一个接受循环里一个个处理，
#             service.rs；后台线程只在收尾时拿一下），一次健康页快照先后拿两次锁、各读一次
#             （startup/health_input.rs：备份作业、仓库抽查）；
#   一次回收（RECLAIM_BUDGET）加一次封存的冻结窗口（一个冻结预算之内；这里按批准的 30 秒算）。关停只等到窗口放开
#   （producer.rs 的 settle_window）：放开之后从定格导出、发布那一段（BUG-045，另给 SEAL_BUDGET）关停不等，
#   进程退出时丢下，和「封存做到一半被杀」同一条回收路，所以这一项不用加。
# Bridge 段 = 同步审批的 stop_grace_seconds（169 现行 30）+ 60（deploy/sync-bridge/README.md 的要求）。
# 后面两项是余量。UNBUDGETED_STEPS_SECONDS 只给不受预算管的小步骤：建暂存区、写准入或失败结果。
# STOP_SAFETY_MARGIN_SECONDS 是保险余量，给下面没算进去的几样和还没数到的路径。
#
# 前面各项按上限相加是预算（185 秒），不是保证；程序里也没有一个关停总期限兜着。下面这些没有逐项算：
#   - 读账本是读完一页才看预算，单次可能多出一页的时间；
#   - 后台线程收尾时要拿一下状态锁把 busy 复位，关停时会跟着等健康页的那次读完；同时在途的健康页快照也可能不止一个。
#   - 后台读账本的线程（producer/catch_up.rs，T-219：监听起来就把账本读到末尾，读完就退出）：关停先置停止标志，
#     它读完手上这一页（8192 条；70 万条账本时一页约 0.1～0.2 秒，跟着账本总量涨）就停。它在状态锁之外读账本，
#     拿锁只做内存里的记账，不占上面「等锁时别人读」那一项。
# 小步骤那 10 秒吸收不了这些，靠保险余量兜。保险余量取 105 秒的来历：这个数经过四轮独立复查，每轮都查出一条漏算的路径
# （Bridge 段、处理请求、等锁时别人读账本、排着的连接），最大的一条 95 秒；105 秒够再漏一条同样大的，
# 外加后台线程收尾等锁的 10 秒。合计 300 秒是用户 2026-11-20 定的。代价：人侧真挂死时要等满 300 秒才被强杀。
# 各项很少同时顶到上限（读账本的各处共用一个读位置，别人读掉的积压请求就不用再读，请求自己的第二、三次读只读
# 上一次读完之后新追加的，一般就是它自己刚写的几条），所以实际用时通常远小于这个数。
#
# 超时不是无害的。systemd 到点强杀时关停链还没走完：人侧 HTTP 入口（/run/studiolo-human/http.sock）和黑板入口
# （BOARD_SOCKET）要到最后、排空 HTTP 和停黑板之后才删（serving.rs 的 drain、unix_ingress.rs），单元又是
# RuntimeDirectoryPreserve=yes，两个节点都会留下。留下之后人侧下次启动按设计拒绝，激活脚本停服后的核对
# （activation_ops.Operations.stopped）也拒绝，激活和回滚都停下来等人看过再删。备份自己这边：socket 在接受循环结束时
# 就删了；封存做到一半被杀，留下的原意图下一次 schedule 补失败结果，暂存一般由下次启动的第一轮回收收走，但正好在
# 「建目录」和「登记落盘」之间被杀的话回收会失败关闭、之后的封存全部失败，也要人处理。
# 所以这个数宁大勿小；想调小保险余量之前，先看上面没逐项算的那几样。
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
STOP_SAFETY_MARGIN_SECONDS = 105
# 10 + 5×(3+2) + 30 + 30 + 30 + 60 = 185，再加 10 + 105 = 300；test_assembly 钉这两个结果，任一项改了要一起看。
STOP_TIMEOUT_SECONDS = (BACKUP_CONNECTION_WINDOW_SECONDS + BACKUP_SCAN_BUDGET_SECONDS * BACKUP_REQUEST_SCANS
                        + BACKUP_RECLAIM_SECONDS + BACKUP_FREEZE_BUDGET_SECONDS
                        + BRIDGE_STOP_GRACE_SECONDS + BRIDGE_STOP_MARGIN_SECONDS
                        + UNBUDGETED_STEPS_SECONDS + STOP_SAFETY_MARGIN_SECONDS)
# 人侧单元的内存上限（BUG-054）：封存要拷 693 MB 的账本，人侧常驻又锁着约 290 MB 内存，512M 装不下——
# 2026-11-24 拷账本时 cgroup 内存耗尽被杀、停站 31 分钟。程序这一版把没落盘的量封顶，单元同时放宽到 2G，
# 免得一份封存反复从盘重读（放宽后放开之后那一段 280 秒降到 107～132 秒，同日 169 上量过）。
# 现场必须恰好一行旧值：多一行、少一行、值不对都不出候选。
OLD_MEMORY_LINE = 'MemoryMax=512M\n'
MEMORY_LINE = 'MemoryMax=2G\n'
# 固定摘要防止覆盖其他发布。两个都是 2026-11-24 从 169 只读回读的现场值：human 是 r23 激活写下的单元
# （已接备份：两个写根、停止超时 300 秒；和 r23 发行目录里那份逐字节相同）；agent 是 T-215 部署之后的单元
# （发布名 t215-116f5d7-20261120），r23 没动它，和上一版钉的一致。
HUMAN_SHA = '2953684239afc7d03ec41106b70b68a1a9d7ef2c96c0211f91d4184be01242f6'
AGENT_SHA = '394ffe6f55823e7ac8cf7cad02fcb2c34cbee0d284afd3c706d13afe54f50117'
# 键名沿用早期协议，内容钉住当前 r23 manifest（2026-11-24 从 169 只读回读）。
R8_MANIFEST_SHA = 'e41276cdb6860bd211cb2ef877d8bbfef42a6b6c314ecf90476413bf562c94a2'
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
    """上一发行的人侧单元已带黑板、Bridge 与备份接线；本版指到新发行、放宽内存上限，其余必须逐字不变。"""
    if text.count(OLD_RELEASE) != 2 or RELEASE in text:
        raise ValueError('unrecognized human unit')
    if 'RuntimeDirectory=studiolo-human studiolo-board\n' not in text:
        raise ValueError('board wiring missing')
    for path in (VAULT, WRITER, BOARD, AGENT, BOARD_RUNTIME, BRIDGE_ROOT, TRANSPORT_ROOT):
        if 'ReadWritePaths=' + path + '\n' not in text:
            raise ValueError('board wiring missing')
    # 上一发行（r23，接线沿用 r17）已接备份：两个写根各恰好一行、紧跟在 Bridge 传输根后面，别处不再提备份路径；
    # 停止超时恰好是按预算算出来的那一行。少了、多了、值不对，都说明现场不是钉住的形状。
    wired = 'ReadWritePaths=' + TRANSPORT_ROOT + '\n' + ''.join(
        'ReadWritePaths=' + path + '\n' for path in (BACKUP_RUNTIME, BACKUP_STAGING))
    if (text.count(wired) != 1 or text.count('studiolo-backup') != 2 or text.count('TimeoutStopSec=') != 1
            or text.count(stop_timeout_line(STOP_TIMEOUT_SECONDS)) != 1):
        raise ValueError('backup wiring missing')
    # 内存上限恰好一行旧值，独占一行（前面是换行）；只换这一行。
    if text.count('MemoryMax=') != 1 or text.count('\n' + OLD_MEMORY_LINE) != 1:
        raise ValueError('memory limit changed')
    return text.replace(OLD_RELEASE, RELEASE).replace(OLD_MEMORY_LINE, MEMORY_LINE)


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
    """沿用黑板、Bridge、只读身份目录和备份配置的接线，只换发行路径；仅生成候选，不激活。"""
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
    # 上一发行（r23，接线沿用 r17）已带备份配置参数：恰好一处，紧跟在同步审批后面，路径是写死的那一份。
    approval = '"--sync-transport", "' + SYNC_APPROVAL + '",'
    if (text.count(approval + ' "--backup-config", "' + BACKUP_CONFIG + '",') != 1
            or text.count('--backup-config') != 1):
        raise ValueError('backup wiring missing')
    text = replace_once(text, 'RELEASE = "' + OLD_RELEASE + '"',
                        'RELEASE = "' + RELEASE + '"')
    if OLD_RELEASE in text:
        raise ValueError('stale release path retained')
    compile(text, str(Path(RELEASE) / 'launch-human.py'), 'exec')
    return text
