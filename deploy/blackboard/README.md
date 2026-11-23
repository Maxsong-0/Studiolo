# 正式 canonical 与黑板首次接线

这是绑定既有 `m4-t094-20260920-r8` 的一次部署，不是通用安装器。只为已有生产 Auth、Audit、Policy 和 agent 身份库增加 canonical/Broker/BoardPlane；不签发身份、不发布权限、不启用 Runner 或模型 API。任何已绑定文件、目录、单元或摘要变化都先停在预检，不覆盖其他发布。

## 构建与候选包

在 GithubCI 原生 Linux 完成 workspace fmt、严格 Clippy、全量测试、路径安全及生产特征构建：

```sh
cargo build --locked --release -p studiolo-vault-admin
cargo build --locked --release -p studiolo-local-server --features studiolo-auth/production
python3 deploy/blackboard/test_assembly.py
python3 deploy/blackboard/test_freeze.py
python3 deploy/blackboard/test_activation.py
python3 deploy/blackboard/test_sync_tools.py
python3 deploy/blackboard/test_sync_release.py
```

初始化工具不依赖 Auth，不具备测试因子验证器或 Auth 的 `production` feature；不要给它附加 `studiolo-auth/production`。人侧使用包含纯黑板能力集修复的新 production 构建；Auth 管理程序与网页仍复用已验证的 r8 文件。

生产工具目录保持仓库相对布局：本目录（含 `candidate.py`、`binding.py`、`snapshot.py`、`sync_tools.py`）、`deploy/human/release/{release_common,runtime_checks,activation_ops,snapshot_worker}.py` 和 `scripts/{replica_common,replica_store,replica_cleanup}.py`。目录及祖先必须真实、root 或 UID1000 所有、不可组/其他写；文件单链接。部署到专用路径，不从 `/tmp` 或另一个项目的工具目录运行。

（历史：r2 首次接线时）UID1000 运行当时版本的 `prepare.py --source-commit <初始化程序来源提交> --vault-admin <初始化程序绝对路径> --vault-admin-sha256 <摘要> --human-source-commit <人侧程序来源提交> --local-server <人侧程序绝对路径> --local-server-sha256 <摘要>`，生成固定 `assembly.RELEASE` 的全新私有 r2 候选包。**这组参数已废弃**，现行用法见文末「现行发行（rN→rN+1）」。它替换人侧程序、派生启动参数和两个服务单元，并加入初始化程序；Auth 管理程序与网页必须仍逐字节匹配 r8。manifest 分别记录两份程序的来源，不能将复用程序冒称为新提交构建。未完成候选保留 `INCOMPLETE`，不自动清理或重试覆盖。

r1 候选从未激活；它已完成一次 canonical 首建。r2 沿用同一已初始化的 canonical/Broker 绑定，不再次执行冻结或初始化。旧 r1 候选、冻结快照和首建审计原样保留。旧 agent 是 root:maxsong 的 0550 冻结程序；预检核验这一既有模式，不修改它的权限。

## 冻结与首建

1. 保留现有 Vault→agent 只读副本同步。UID1000 运行 `python3 -I -B -S …/freeze_replica.py`；它在原同步锁内核验 ready、300 秒新鲜度、三个批准范围和所有字节，写入全新私有快照。共享锁竞争、陈旧副本或不完整内容均拒绝。
2. 事先建立 `assembly.VAULT`、`assembly.WRITER`、`assembly.BOARD` 三个 UID1000/0700 空目录。禁止将轮换副本原地升级成写者。
3. 用新 `studiolo-vault-admin plan` 指向现有 owner 配置、candidate web、冻结快照中的 **`vault/` 子目录**和两个首建目录；快照外层含同步清单，不是 Vault 根。两个项目使用 `--allow 项目/Studiolo --allow 项目/Tillerwright`；`_规则` 含 ZIP/TXT 附件，必须逐一指定其 10 份 Markdown 的精确路径，不得批准整个 `_规则` 前缀。这次共 12 个范围、207 份 Markdown；四份附件仍保存在原冻结快照和原只读副本中，不导入 canonical。审阅文件数、字节数及 plan 摘要后，以完全相同参数加 `--expected-plan` 执行 `initialize`。两个命令都要求原生 Linux 进程加固成功，包括 unlimited MEMLOCK。
4. 首建通过 Broker 写入 Markdown，并记录 `local_os_bootstrap` 审计；不冒充人类 L3 或 Sync 证据。部分首建失败原样保留，禁止盲目再次初始化。

来源只被冻结一次。原 Obsidian 的后续变化仍进入旧只读副本，不会自动进入新的 canonical。网页标签明确显示“初始快照（持续同步未接通）”；新库不提供正文编辑所需的同步新鲜度证明。

## 双服务切换与验收

使用 root 执行固定工具：

```sh
sudo /usr/bin/python3 -I -B -S /可信工具目录/deploy/blackboard/activate.py \
  --manifest-sha256 <候选 manifest 的实际摘要>
```

预检要求两个原单元摘要、enabled/无 drop-in、实际旧进程、v2 认证绑定、完整候选文件集合及 Broker 配对均准确。`/var/lib/studiolo-deployments` 必须为 root 所有、无组/其他写权限的真实目录；保留既有 0750 总目录和历史记录的访问设置，本次子记录仍为 root/0700、锁及快照文件为 root/0600。顺序为停止 agent→停止 human→验证 PID 和 socket 均已退出→三库一致快照→替换单元→启动 human 并验证两个私有 socket→启动 agent。绝不删除存量 socket。

认证、恢复码计数、会话 epoch、owner 配置以及既有 agent 程序和三个读取范围不变。回执明确区分服务切换成功和尚未验收的权限/本机客户端。停止后故障仅在状态仍属于本次已知配置时恢复旧程序，不回滚数据库；未知认证/单元变化保留现场，标为 `blocked_requires_operator`。断电、SIGKILL 或恢复过程失败须依据私有发布记录人工处理。

切换后还须验证真实 MCP 黑板读取、未授权写入拒绝、原有读取新鲜度、人侧权限草稿与实际 executable。主人亲自完成 Claude/Codex 的 L3 发布后，才能验证授权写入、项目越权拒绝、持久化、撤权，以及两个本机 Consumer 的 claim/heartbeat/return 完整回执。工具测试、候选包、服务启动都不能代替这些生产验收。

## 现行发行（rN→rN+1）

r2 之后每次发行只做「上一发行 → 本发行」：换网页，可选换人侧程序，其余逐字节沿用（`assembly.py` 文件头有沿革，r5→r6 见 T-159）。

1. **改常量**（`assembly.py`）：`OLD_RELEASE`＝上一发行目录、`BUILD`＝新发行号；`HUMAN_SHA`／`AGENT_SHA` 取 169 现场 `/etc/systemd/system/studiolo-{human,agent}.service` 的 sha256；`R8_MANIFEST_SHA` 取上一发行 `manifest.json` 的 sha256。`test_assembly.py` 的发行号断言同步。三个变换函数通常不改；本发行要动启动器参数、单元写根或停止超时（如 r17 接备份）时，改对应的变换函数和它的测试，新前置放进激活预检，新参数旧程序不认的，把参数名的字节字面量写进 `NEW_HUMAN_PROGRAM_LITERALS`（`REQUIRES_NEW_HUMAN_PROGRAM` 由它派生，不直接改）；下一发行不再要求换程序时清空它。
2. **服务器上验工具**：`scripts/test-linux.sh bash -c 'python3 deploy/blackboard/test_assembly.py && python3 deploy/blackboard/test_freeze.py && python3 deploy/blackboard/test_activation.py'`（本机不跑测试）。
3. **构建**：人侧程序在 githubci 原生 `cargo --locked build --release -p studiolo-local-server --features studiolo-auth/production`；网页在 108 用同一提交 `npm ci && npm run build`。来源提交须已过 CI。
4. **暂存**（169，UID1000，0700/0600，程序 0755，不用 `/tmp`）：`bin/studiolo-local-server`、`web-dist/`、本目录 `{activate,activation_ops,assembly,prepare,candidate,binding,snapshot,sync_tools}.py` 与 `deploy/human/release/{activation_ops,release_common,runtime_checks,snapshot_worker}.py`，保持仓库相对布局。
5. **出候选**（UID1000）：

```sh
/usr/bin/python3 -I -B -S <暂存>/deploy/blackboard/prepare.py \
  --web <暂存>/web-dist --web-source-commit <40 位提交> \
  --local-server <暂存>/bin/studiolo-local-server --local-server-sha256 <64 位摘要> --human-source-commit <40 位提交>
```

   只换网页时省略后三个参数（三件套要么全给要么全不给），程序逐字节沿用上一发行、manifest 照抄其来源；`assembly.REQUIRES_NEW_HUMAN_PROGRAM` 为真的发行（如 r17）必须给，而且给的程序要和上一发行字节不同、字节里有本版新参数的字面量（`NEW_HUMAN_PROGRAM_LITERALS`），否则不出候选。T-132 r10 同时传入 `--maintain <程序> --maintain-sha256 <摘要> --sync-launcher <程序> --sync-launcher-sha256 <摘要>`；四项必须全给，并与上述人侧程序来自同一提交。manifest 新增 `sync_program_source_commit`，两个程序均为 ELF/0755；之后省略四项会原样保留，禁止静默移除、缺一程序或保留来源却换字节。工具随包交付不自动置备、配对或启用 Bridge。输出 `manifest_sha256`、`human_binary_reused`、`services_changed: false`。
6. **激活**（用户本人 root）：`sudo /usr/bin/python3 -I -B -S <暂存>/deploy/blackboard/activate.py --manifest-sha256 <上一步的摘要>`。已激活后重复执行会在预检被拒（现场已不是上一发行），无副作用。

**r15→r16 是单向的**（2026-11-16 部署前复查）。r16 一启动就把控制库（黑板目录里的 `control.sqlite3`）从
schema 3 升到 4，r15 见到比 3 新的版本拒绝打开；启动时写者若有没收口的操作，写者状态升到格式 8，r15 只认 1～7；
做过新根恢复的库还会出现 lineage 第 3 版。激活前那份三库一致快照只含 auth、audit、policy，不含控制库和写者状态。
所以 r16 启动过之后激活再出故障，`activate.py` 换回的 r15 程序也起不来：只能前滚，查明原因修好后重新激活
（现场换回 r15 后预检允许再激活）。要留退路，就得在激活前停掉两个服务、把人侧全部数据根原样拷一份恢复点，再起服务、
立即激活；退回时恢复点连同 r15 程序一起换回，恢复点之后的写入全部丢失，Bridge 状态也一起退回。做不做、怎么拷，
按当次交接档，由用户在场定。


上一版固定 r11→r12（2026-10-01）：修复 BUG-021 的退出观察顺序与 Stop 提前销毁证据。
人侧程序和 Bridge 双程序来自同一修复提交；网页沿用 r11 已验证字节与来源 `4a1ba63`，agent 沿用 t160。
当前 r11 已启用 Bridge，发行将现场已有的标签、`--sync-transport` 与两个写根纳入包，权限范围不变；
人侧 ExecStart 从 P4 外部启动器迁回新发行内。现场单元和上一 manifest 摘要仍精确钉在 assembly.py。
root 安装的 sync-launcher 也必须按新构建摘要单独升级，不能只交付到发行目录就冒称运行体已更新。
已有 LaunchIssued/container=null 现场仍只能凭真实 boot 变化恢复，不清状态、不重新初始化或配对。
激活保留认证、数据库、配对和同步范围；程序发行不等于 P4 首轮导入或 P5 两周试跑验收。


上一版固定 r12→r13（2026-10-01）：纳入BUG-022目录初始化维护、BUG-023持久保全4096项上限和BUG-024批内安全关停与BUG-025陈旧未发预约取消。
既有r12启动参数、标签、写根和认证辅助函数逐字沿用，只替换发行路径。网页和agent沿用既有字节；不更新root launcher或再次整机重启。
r12本身尚无批内关停点，激活前必须取得空闲plane.lock并持有到旧服务正常退出，再走官方有界切换；不能在长批次中直接stop。
不迁移或清空Bridge计划、基线和stage；新程序按持久原键续做。发布成功仍不代表P4导入验收。

上一版固定 r13→r14（2026-10-01）：BUG-026只增加同步失败步骤与固定错误类别日志，不改变同步协议和权限。
人侧程序更新，网页、agent和root launcher字节沿用；r13路径/单元/manifest钉住后按官方事务切换，仍须持空闲plane.lock避免新轮插入。

上一版固定 r14→r15（2026-10-01，已激活）：BUG-027将Bridge文件检查改为不跟随链接的元数据查询，避免关闭临时FD释放SQLite进程锁。
数据库版本及外部写入检测保持；只替换人侧程序和发行路径，沿用r14网页、agent、root launcher与认证/同步配置，使用同一空闲plane.lock切换流程。

本版固定 r15→r16（2026-11-18，T-207）：发行 main 上 BUG-038 项目目录改名与 T-207 各批人侧改动；启动器加 `--agent-data`，
装配要求 agent 单元的读取范围已含新名（`AGENT_READ_SCOPES`）。人侧程序、网页、maintain 与 sync-launcher 一起换，`prepare.py` 带全部参数；
控制库 schema 3→4、单向（见上文「r15→r16 是单向的」），要留退路先做恢复点。现场摘要：human 单元 `edb0c1f5…c71c7`、agent 单元 `15b39395…45b9`（t207-c09b065、新名 `--allow`）、r15 manifest `2f24ad58…a6d5`。

r16→r17（2026-11-20，T-216，已激活）：启用备份生产者。启动器在同步审批后面加 `--backup-config`（和主人配置同目录的 `backup.json`），
人侧单元加备份 socket 父目录、封存父目录两行 `ReadWritePaths=`，停止超时 90→300 秒（备份收尾和 Bridge 停止是串行的两段，逐项相加是 185 秒，另加 115 秒余量，
算法和余量的来历在 `assembly.py` 的常量注释里）；这三处由装配一次换上，激活失败一起退回。激活脚本停服务的命令时限按停止超时加 20 秒算（320 秒）。
**激活前要备好**（`deploy/backup-worker/README.md`「安装前要有的东西」）：读取组和人侧账号的组成员、tmpfiles 建的两个目录、
`backup.json`、临时数据根。缺读取组成员、两个目录或 `backup.json`，人侧起不来；缺临时数据根人侧照常起，第一次封存才失败。
两类都由激活预检在停服之前拦下（`activation_ops.backup_ready`，另核 `backup.json` 的键名和两处 `version`、备份 socket 没有残留、
冻结预算和 Bridge 停止宽限在程序接受的下限和装配假设的上限之间）：预检不过就不停服务，现场不动。`backup.json` 各字段的取值规则
（审批段内容、worker 账号、保留期、清单、路径互不嵌套等）预检不重核，只由程序启动时核：写错了会在停服之后启动失败、退回 r16。
人侧程序必须一起换（上一发行的程序不认 `--backup-config`；`REQUIRES_NEW_HUMAN_PROGRAM` 为真，出候选和预检都要求候选程序
和 r16 字节不同、字节里有 `--backup-config` 这个字面量，只填一个新的来源提交号过不了），网页、maintain 与 sync-launcher 也一起换，
`prepare.py` 带全部参数；agent 单元沿用 T-215 换过发布名之后的现场。r16 之后没有改数据库或状态的版本号，激活失败可以退回 r16（退回后启动器不带 `--backup-config`，`backup.json`
留着不碍事）。人侧要是被强杀的（停服超过停止超时也算），`/run/studiolo-human/http.sock`、黑板入口 `/run/studiolo-board/board.sock`
和 `/run/studiolo-backup` 里的 socket 节点都可能留下：前两个会让激活脚本停服后的核对当场拒绝，激活和回滚都停下来等人处理；
后一个在重试激活时被预检拒。都要人看过再删，步骤在 `deploy/human/RECOVERY.md`。
装配钉的摘要：human 单元 `11827335…a143`（r16 激活写下的，2026-11-20 回读的现场值）、r16 manifest `437d75a8…7b4e`（同上）、
agent 单元 `394ffe6f…0117`（**预算值**：线上单元里的发布名两处换成 `t215-116f5d7-20261120` 之后应有的摘要，换之前是
`15b39395…45b9`；T-215 的 agent 换完、回读一致才算现场值，不一致就改 `AGENT_SHA` 重出候选）。
下一发行的起点已经带备份监听：预检里「备份 socket 必须不存在」那一条届时要改成核活的 socket，「必须不存在」挪到停服之后（r18 已照此改）。

r17→r18（2026-11-21，T-219，已激活）：备份首跑三处缺陷的修复里人侧的两处，BUG-042（重启后备份队列自己把账本读到末尾、
账本首尾查询不再扫整本）和 BUG-044（封存排空之前先补记登录库）。**只换人侧程序**，启动器和人侧单元除了发行路径一个字不动：
`--backup-config`、备份的两个写根、停止超时 300 秒都沿用 r17，装配核它们各恰好一处、位置和值都对，不对就不出候选。
人侧程序必须换（`REQUIRES_NEW_HUMAN_PROGRAM` 为真）：候选程序要和 r17 字节不同、字节里有 `备份封存卡在：` 这几个字
（本版新加的一句日志的开头；169 上 r17 的程序里没有，2026-11-21 只读核过），只填一个新的来源提交号过不了。
网页、maintain 与 sync-launcher 一起换，`prepare.py` 带全部参数；agent 单元原样沿用。没有改数据库或状态的版本号，激活失败可以退回 r17。
起点 r17 已经在监听备份 socket，所以预检改了两处：`backup.json` 里的 socket 必须就是 `/run/studiolo-backup/producer.sock`，
而且此刻是活的那一个（socket 节点、主服务账号、读取组、0660、单链接）；停服之后的核对多看一样，它必须已经删掉
（和人侧 HTTP 入口、黑板入口一样，留下就当场拒绝，等人看过再删）。
**激活前还要做的**：worker 程序和私有备份配置跟着发行走，先停 worker 和定时器、跑 `deploy/backup-worker/provision/prereq.py --apply`
把这两样从 r17 换成 r18（上一版留 `.before-r18` 副本），再激活；顺序和退回办法在 `deploy/backup-worker/README.md`「换发行」。
装配钉的摘要（都是 2026-11-21 从 169 只读回读的现场值）：human 单元 `92db3c46…9058`（r17 激活写下的）、r17 manifest `8aef1b27…faeb`、
agent 单元 `394ffe6f…0117`（和上一版预算的一致）。

r18→r19（2026-11-22，T-222，已激活）：BUG-046，主库没开文件操作时，Bridge 碰到主库删除就整批暂停，不再记成「终态未知」
（169 的同步从 2026-10-02 起卡在计划 11，封存冻结 Bridge 也一直被它挡住）。**只换人侧程序**，启动器和人侧单元除了发行路径一个字不动，
装配的核对和 r18 一样。人侧程序必须换：候选程序要和 r18 字节不同、字节里有 `studiolo_bridge step=DeleteGate outcome=deletes_blocked count=`
（本版新加的一行同步日志到占位符为止；169 上 r18 的程序里没有，2026-11-22 只读核过）。
网页（健康页同步节多一行「删除被挡」）、maintain 与 sync-launcher 一起换，`prepare.py` 带全部参数；agent 单元原样沿用。
没有改数据库或状态的版本号（Bridge 库不加表、不改编码），激活失败可以退回 r18。
**激活前还要做的**：和 r18 一样，先停 worker 和定时器、跑 `prereq.py --apply` 把 worker 程序和私有备份配置从 r18 换成 r19
（上一版留 `.before-r19` 副本），再激活。装配钉的摘要（都是 2026-11-22 从 169 只读回读的现场值）：human 单元 `d8da14e8…49e0`
（r18 激活写下的）、r18 manifest `55c39dd4…6ae7`、agent 单元 `394ffe6f…0117`（没变）。

本版固定 r19→r20（2026-11-22，T-226，合版带 T-224）：BUG-047，同步一轮能比对的路径数从 4096 提到 16384
（169 的三方并集已到 4108 条，r19 每轮观察都失败），健康页同步节加「路径用量」「比对内容」两项预警；
T-224 的同步状态回执 `sync-status.json` 也从这一版起写。**只换人侧程序**，启动器和人侧单元除了发行路径一个字不动，
装配的核对和 r19 一样。人侧程序必须换：候选程序要和 r19 字节不同、字节里有四段新字节
（`studiolo_bridge component=capacity from=`、`studiolo_bridge step=Observe outcome=path_budget_exceeded used=`、
`studiolo_bridge step=Approve outcome=byte_budget_exceeded used=`、`sync-status.json`；169 上 r19 的程序里各 0 处，2026-11-22 只读核过）。
网页（健康页同步节多两行）、maintain 与 sync-launcher 一起换，`prepare.py` 带全部参数；agent 单元原样沿用。
**激活前还要做的**：和 r19 一样，先停 worker 和定时器、跑 `prereq.py --apply` 把 worker 程序和私有备份配置从 r19 换成 r20
（上一版留 `.before-r20` 副本），再激活。装配钉的摘要（都是 2026-11-22 从 169 只读回读的现场值）：human 单元 `5eaacbbb…15e2`
（r19 激活写下的）、r19 manifest `4df263c1…9e7b`、agent 单元 `394ffe6f…0117`（没变）。

**r19→r20 对 Bridge 库是单向的**。Bridge 库不加表、不改编码，但 r20 的人侧一启动就把库里存的路径预算从 4096 升到 16384
（服务日志有一行 `studiolo_bridge component=capacity from=4096 to=16384`）；r19 打开这样的库判预算超限，人侧直接起不来、不监听。
`activate.py` 失败时只换回程序、不动数据库，所以 r20 启动过之后激活再出故障，换回的 r19 也起不来。要留退路：激活前停人侧，
把 Bridge 库目录 `/var/lib/studiolo-owner/bridge/plane/bridge` 整份拷一份，再起人侧、立即激活；退回时停人侧、换回这份拷贝、再起 r19
（拷贝之后 Bridge 记下的东西全部丢失；r19 本来每轮都在观察阶段失败，这段时间不会有新的批次）。做不做、怎么拷，按当次执行单，由用户在场定。
