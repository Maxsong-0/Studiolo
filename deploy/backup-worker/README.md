# 独立备份 worker 的部署模板

T-204。本目录是候选发布源，**尚未安装到任何机器**。专用账号、读取组、目录、固定 restic 包和仓库凭据
都要用户本人建好并逐项验收；测试箱上的结果不能代替这一步。契约见
[`docs/contracts/backup-producer.md`](../../docs/contracts/backup-producer.md) 和
[`docs/contracts/backup-worker-process.md`](../../docs/contracts/backup-worker-process.md)。

## 三个角色

| 角色 | 账号 | 做什么 | 拿不到什么 |
|---|---|---|---|
| 主服务（生产者） | 人侧服务原来的账号，加进读取组 | 发序号、冻结并封存、写准入、答队列 | 仓库凭据 |
| worker（`studiolo-backup-worker run`） | 专用账号 `studiolo-backup`，加进读取组 | 取待办、读封存、追加上传、核对、写结果；空闲时每天抽查仓库的一组 | 账本文件、任何 owner 的库 |
| 定时触发（`… schedule`） | 同 worker 账号 | 每五分钟请主服务判断一次该不该封存 | 凭据、正文 |

读取组（示例名 `studiolo-backup-read`）只用来让 worker 读最终封存和连备份 socket。

## 程序的三个子命令

没有别的参数，也不读环境变量：

- `run`：常驻。启动先收紧秘密驻留面、读 `/etc/studiolo/backup-worker.json`、打开状态目录，把上次没做完的阶段标成
  未知；之后每 30 秒问主服务「下一个待办」，对它做一步。没有待办时问主服务要不要抽查仓库的一组
  （`check --no-lock --read-data-subset <n>/30`，30 天一圈，见
  [`docs/contracts/backup-repository-check.md`](../../docs/contracts/backup-repository-check.md)），抽查时来了作业就让路。
  子进程清理没能证实时退出，由 `Restart=on-failure` 拉起。
- `schedule`：一次性。输出 `started`、`outstanding` 或 `not_due` 之一，三种都算成功；连不上主服务才失败。
- `init`：显式初始化 `/var/lib/studiolo-backup-worker/state`。已经有锁或状态就拒绝，不会重置序号。

## 安装前要有的东西

169 和 NAS 上的这几项有现成的置备脚本，在 `provision/`（都先预演，带 `--apply` 或 `add` 才动手；sudo 和口令录入由用户本人做）：

- `prereq.py`：第 1～4、6 项和三个单元。已经对的跳过，在但不对的停下不覆盖；两份配置由它生成，生成的样子各留了
  一份夹具，由程序的真实解析读过（`crates/host-config/tests/support/provisioned-backup.json`、
  `crates/backup-worker/tests/support/provisioned-config.json`）。
- `nas-account.command`（在 Mac 终端跑，NAS 一侧是 `nas_account.py`）：给 worker 在 NAS 的 rest-server 上加只追加账号。
  它的 `unit` 用法换 NAS 上备份服务的单元：原来那版在开机时存储卷还没挂上就被跳过、之后一直不起来
  （2026-10-24 那次开机之后停了四周，没人发现），新版没准备好会每 5 秒再试。worker 上线之前要换好，
  不然 NAS 一重启备份就全部失败（见 `deploy/sync-bridge/recovery/README.md`「NAS 追加备份」）。
- `credentials.py`：第 5 项。不回显地录入两个口令，写三份凭据文件，再以 worker 账号初始化仓库并读回。

顺序：NAS `check` → NAS `add` → `prereq.py --apply` → `credentials.py --apply`；NAS `unit` 在 worker 上线之前哪一步做都行。
人侧发行换了候选、worker 重建过，先改 `prereq.py` 里「发行相关的值」那一段。下面是这几项各自的要求（手工做也照这个）。

1. 专用账号 `studiolo-backup` 和读取组；主服务账号和 worker 账号都在读取组里。
2. `/usr/lib/studiolo-backup-worker/`（root 所有、不可组写）里放：
   - `studiolo-backup-worker`：在 108 用 `cargo --locked build --release -p studiolo-backup-worker` 构建；
   - `restic-0.19.1`：官方发布包里的可执行文件，摘要写进配置的 `restic_sha256`。本仓库不下载它。
3. `/etc/studiolo/backup-worker.json`（root 所有，0644 或 0444），字段见进程边界契约；`producer_socket` 和
   `sealed_parent` 要和主服务私有备份配置 `backup_producer` 段里的 `socket`、`staging_parent` 是同一组路径，
   `restic_sha256` 和 `restic_digest` 是同一个值。
   仓库是自签证书（NAS 上的 rest-server 就是）：把它的 CA 证书放到 `/etc/studiolo/backup-worker-ca.pem`（root 所有，0644），
   在配置里写 `repository_ca_sha256`（这份文件的 `sha256sum`）。摘要对不上 worker 拒绝启动；不改 169 的系统信任库。
4. `systemd/studiolo-backup.tmpfiles.conf` 里的两个目录（属主主服务账号、属组读取组、0750）。主服务不建它们。
5. `/etc/studiolo/backup-worker-credentials/`（root 所有，0700）里三个文件（0600）：`repository-password`、
   `rest-username`、`rest-password`。由用户经安全输入写入，不从聊天或日志里复制；单元用 `LoadCredential` 交给
   worker，worker 只从 `/run/credentials/studiolo-backup-worker.service` 读，所以**服务单元名不能改**。
6. 主服务私有备份配置：和主人配置同目录（0700）放一份 `backup.json`（0600，主服务账号所有），内容
   `{"version": 1, "backup": {…}, "backup_producer": {…}}`，两段成对，字段见 `docs/contracts/backup-producer.md`；
   人侧启动器加 `--backup-config <这份文件的绝对路径>`。**不要把两段写进 owner.json**：那会改 Bridge 绑定的
   `scope_hash`，人侧起不来（owner.json 里出现这两段会在加载时被拒）。主服务单元（`deploy/human/studiolo-human.service`）
   同时改两处，再重启主服务：
   - 单元是 `ProtectSystem=strict`：给备份 socket 的父目录和封存父目录各加一行 `ReadWritePaths=`（路径和第 4 步
     tmpfiles 片段里的一致）。不加的话绑不上 socket、建不了封存，配了生产者的主服务会拒绝启动。
   - 停止超时：主服务收到退出信号先等备份收尾，做完才通知同步 Bridge 停，两段是串行的，要相加。
     备份段等的是在途的那一个连接整个处理完，再等它起的后台线程：收发窗口 10 秒；处理请求不受这个窗口管，
     要拿状态锁读账本，每次读按 5 秒算：自己最多读三次；等锁时别人读两次（别的会拿着这把锁读账本的只有健康页，
     一次健康页快照先后拿两次锁、各读一次），合起来按 25 秒算；
     一次回收最多 30 秒；一次封存（冻结和导出在一个冻结预算之内，预算上限 120 秒）。
     Bridge 段 = 同步审批的 `budget.stop_grace_seconds` + 60 秒（`deploy/sync-bridge/README.md`）。没接 Bridge 的部署只算
     备份段，再留出原来关停链的时间。r16 及之前单元里的 `TimeoutStopSec=90` 不够。
   - 这样逐项相加得到的是预算，不是保证，程序里也没有一个关停总期限。没有逐项算的：读账本是读完一页
     才看预算，单次可能多出一页的时间；后台线程收尾时要拿一下状态锁，会跟着等健康页的那次读完；同时在途的健康页快照
     可能不止一个。所以预算之外要另加保险余量（169 的取值见下）；各项也很少同时顶到上限（读账本的各处共用一个读位置，
     别人读掉的积压请求就不用再读）。「只等在途的那一个连接」靠的是接受循环先看停止信号：信号一到，排着的连接不再接、
     到点的回收不再起（`crates/local-server/src/backup/service.rs` 的 `accept_until_stopped`，同目录 `service_tests.rs` 钉着）。
   - 超时不是无害的，所以停止超时宁大勿小。被服务管理器强杀时关停链还没走完：人侧 HTTP 入口
     `/run/studiolo-human/http.sock` 和黑板入口 `/run/studiolo-board/board.sock` 要到关停链最后才删，单元又是
     `RuntimeDirectoryPreserve=yes`，两个节点都会留下。留下之后人侧下次启动按设计拒绝，激活脚本停服后的核对也拒绝，
     激活和回滚都停下来等人看过再删。封存做到一半被杀：留下的原意图下一次 `schedule` 补失败结果；暂存一般由下次启动的
     第一轮回收收走，但正好在「建目录」和「登记落盘」之间被杀的话，回收会失败关闭，之后的封存全部失败，也要人处理。
     Bridge 那边，`deploy/sync-bridge/README.md` 保证的是执行段被杀后执行器能按原键续做，不保证人侧能自己起来。
   - socket 残留：正常关停的第一步就把备份 socket 删了，停止超时到点时它一般已经不在。会留下节点的是主服务崩溃、
     内存超限被杀，或者还没走到那一步就被强杀；留下之后下次启动按设计拒绝（不清理也不接管），要人看过再删。
   - 上面这几种残留怎么看、怎么确认是死的、删哪一个，照 `deploy/human/RECOVERY.md` 做（含激活脚本停在
     `blocked_requires_operator`、暂存回收失败关闭两种情形）。

   生产 169 上不手改线上单元：启动器的 `--backup-config`、这两行 `ReadWritePaths=` 和停止超时由人侧发行 r17 的装配一次换上
   （`deploy/blackboard/assembly.py`，2026-11-20），激活失败时一起退回。停止超时是 300 秒：备份段 10＋25＋30＋30（批准的冻结预算
   是 30 秒），Bridge 段 30＋60（169 的 `stop_grace_seconds` 是 30），合计 185；另留 10 秒给建暂存区、写准入或失败结果这些
   不受任何预算管的小步骤（`assembly.UNBUDGETED_STEPS_SECONDS`），再加 105 秒保险余量给上面没逐项算的那几样和还没数到的路径
   （`assembly.STOP_SAFETY_MARGIN_SECONDS`，2026-11-20 用户定；来历在 `assembly.py` 的注释里）。代价是主服务真挂死时要等满
   300 秒才被强杀。
   所以第 1～6 项里别的东西（账号和读取组、两个目录、`backup.json`、下面这个临时数据根）要在激活 r17 **之前**备好：
   启动器一带上 `--backup-config`，读不到配置或绑不上 socket，人侧就拒绝启动。激活预检在停服之前核这些
   （`deploy/blackboard/activation_ops.py` 的 `backup_ready`）：私有配置的权限、键名齐不齐和两处 `version`、
   人侧账号在读取组里、两个目录的属主属组和权限、socket 没有残留、临时数据根、冻结预算和 Bridge 停止宽限落在程序接受的
   下限和装配假设的上限之间；缺一样就不停服务。**预检不重核 `backup.json` 各字段的取值规则**（审批段内容、worker 账号、
   保留期、清单里的包和摘要、三条路径互不嵌套、socket 路径长度）：那些只由程序启动时核，写错了要到停服之后启动失败、
   退回上一发行才知道，所以 `backup.json` 写好后先对着 `docs/contracts/backup-producer.md` 的表逐项看一遍
   （顶层和 `backup_producer` 段各有一个 `version`，都是 1）。
   以后把冻结预算或 `stop_grace_seconds` 调大，要改装配里对应的那一项（`assembly.BACKUP_FREEZE_BUDGET_SECONDS`、
   `BRIDGE_STOP_GRACE_SECONDS`），停止超时由各项相加再加两项余量得出（`STOP_TIMEOUT_SECONDS`，`test_assembly.py` 钉算法和结果）；
   激活脚本停服务的命令时限按停止超时再加 20 秒算（现在是 320 秒，`activation_ops.STOP_COMMAND_MARGIN_SECONDS`）。
   - `backup_producer.ephemeral_root` 指的目录也要事先建好：主服务账号私有（属主是主服务账号、0700、不带 ACL），和 socket
     父目录、封存父目录、各数据根互不嵌套。主服务只读打开它，**不用**加进 `ReadWritePaths=`；不要放在 `/tmp`、`/var/tmp` 下
     （单元是 `PrivateTmp=yes`，服务里看不见宿主的这两处），路径上不要有符号链接。169 上用
     `/home/maxsong/.local/share/studiolo-backup-ephemeral`。封存时才打开它，没建的话人侧照常启动，但第一次封存失败
     （169 上激活预检会先拦下）。
7. **上线前在 Mac 上做一次只读核查**（2026-11-16 用户拍板）：脚本经 ssh 送到存放数据的机器上，**必须以主服务账号跑**
   （主服务开着时脚本只认控制库的属主账号，换别的账号控制库这一项不核：它关库那一刻只读连接可能以那个账号建出
   `-shm`，之后主服务打不开库），退出码不是 0 就先别启用备份，把输出报告用户再定。退出码 2 是有一部分没核成
   （路径读不了、账号不对、出了没预料到的错）：输出逐条写明哪部分、为什么，别的部分照样核完、查出的问题照样列出，
   但结论不完整，不能当成「没问题」：

   ```bash
   ssh <主服务账号>@<机器> python3 -I - --vault <Vault 根> --data <写者数据根> --control <control.sqlite3> < scripts/check-backup-preflight.py
   ```

   脚本（`scripts/check-backup-preflight.py`，2026-11-16）只列目录、读元数据和清单，控制库按只读打开、不建 `-wal`、
   `-shm`。它查的都是「每一次整批封存都会因它失败」的现存数据，规则照抄封存代码：
   - 尝试目录 `sources/attempts/<任务>/<尝试>/`：第 1 版或没有原批准的清单、没有清单的目录、清单没列的文件。
     这类目录因为历史版本还在，手工删不掉。
   - 写者数据根里的暂存正文 `artifact-candidate-*.json`：升级前留下的没有原批准。
   - 原子写崩溃留下的临时文件：`.workbench/` 和写者数据根里形状对得上的，重启主服务开库时会挪进隔离区；
     用户目录里的开库清扫不管，要人看过再删。
   - 封存收不了的节点：符号链接、设备、套接字、管道，多个硬链接的文件，别的账号的文件，属主没有读权限的文件
     （目录还要能进入），挂载点，不是 UTF-8 或带控制字符的名字，导出扫描认不了的路径（编码过的分隔符、非 NFC 拼写等），
     层数超上限；Vault 根和写者数据根两棵树合计的目录项数、字节数超上限（数到上限就停），文件数超导出扫描的上限
     （这里只数这两棵树，控制库、审计这些 owner 的文件另算，离上限不到一成时提醒）。
   - 隔离区目录已经在、却不是主服务账号私有的：开库清扫不会往里挪，那里的临时文件要人看过再处理。
     隔离区里已有的东西封存不带，只报个数（文件和目录都算，数到上限就停）。
   - 控制库：停在「提交中」的 attempt，旧格式的终态（升级前的旧内联预留、报告不是第 3 版），以及检查点校验里
     其余各条（发件箱没投递完的可能是瞬时的，过一分钟再跑一次）。旧内联预留现在只挡它自己那个 attempt 的对账
     （2026-11-16），但控制库检查点仍然拒绝导出，所以照样挡封存。
   - 没核的（由封存本身把关，第一次封存失败会写明原因）：历史目录对历史正文的认领、暂存正文的内容和名字是否
     对得上、清单里原批准的内部字段、控制库表结构清单和产物批准的逐条核对、ACL 和安全模块对读权限的限制
     （这里只看权限位）、导出里别的 owner 的文件数。

## 验收顺序

1. `systemd-analyze verify` 三个单元；核对目录和文件的属主、属组、权限。
2. 以 worker 账号跑一次 `studiolo-backup-worker init`，再跑一次应失败（已经初始化）。
3. 手动跑一次 `studiolo-backup-worker schedule`：第一次应输出 `started`，紧接着再跑应输出 `outstanding`。
4. 启动 `studiolo-backup-worker.service`，日志里应出现「备份作业已收口：Succeeded」，人侧账本里这个原键有一条
   成功结果。再打开主服务健康页（登录后 `/api/v1/health`，或工作台「本机健康」）：「备份作业」一节应为
   正常，正文是「最近一次成功是 … 前的第 N 号」，N 就是这个原键的序号；展开「查看备份作业」能看到同一个序号
   和成功时刻，「挂着的作业」为「没有」。「恢复点年龄」一节也应为正常，正文是「最新恢复点 … 秒前的内容，
   已在预算内」：年龄从这次作业封存时的内容截止算起，预算是审批里的 `rpo_millis`（主服务从同一次读账本推出，
   不读 worker 的状态库）。第一次成功之前这一节固定显示「尚未建立灾难恢复点」。
5. worker 空闲时应派出第 1 组抽查：日志里出现「仓库抽查第 1 组：Clean」，健康页「备份仓库检查」一节为正常，
   正文是「最近一次干净的检查是 … 前；首轮还在读，30 组里读过 1 组」。
6. 停掉 worker 再触发一次封存，过了保留期再启动：应记一条 `source_expired` 失败，不重传。
7. 用恢复演练（另一台机器、另一套恢复凭据）读回这一次的快照，对照封存清单。新根恢复现在整次拒绝（见
   `docs/contracts/new-root-restore.md`），这一步只做到读回并核对为止。

每一步的输出留证；第 4～7 步要真实仓库，测试箱做不了。

## 现在怎么发现作业卡住了

先看主服务健康页的「备份作业」一节（2026-11-16 接上）。它只读人侧账本里的备份事件，阈值来自私有备份配置（`--backup-config` 指的那份）：

| 健康页显示 | 什么意思 | 先查什么 |
|---|---|---|
| 降级：「第 N 号作业挂了 … 还没收口，超过保留期 …，算卡住」 | 有原意图、没有结果的作业挂得比 `source_retention_millis` 还久；展开明细能看到它封存了没有 | 封存过了（「已封存，等 worker 上传」）：看 worker 单元在不在跑、日志里是不是反复「备份 worker 等下一轮」或「备份作业这一步被拒绝」；没封存：看主服务日志里「备份封存未完成」 |
| 降级：「… 超过两倍最小间隔 …，算逾期」 | 离最近一次成功超过两倍 `schedule_interval_millis`；从没成功过的从开始服务和第一次尝试里较早的那个算起 | 定时器在不在跑（worker 没触发 `schedule` 就什么都不会发生），以及最近失败的原因 |
| 降级：「最近收口的第 N 号作业没成功：…」 | 最近一条作业记了失败或未验证；原因是账本里的固定类别（如「磁盘空间不够」「封存前来源变了」）。写着成功、却没写明对象齐全、校验通过、成对引用这次检查点三项的，也算没成功 | 按原因处理；失败之后要等满批准的间隔才重试，失败不缩短间隔（2026-11-16 用户拍板） |
| 未知：「备份生产者没有启用」 | 人侧启动器没有带 `--backup-config` | 是不是有意不启用 |
| 未知：「这一次没读到账本里的备份事件」 | 账本没在扫描预算（5 秒）里读完，或本地时钟不可用 | 刷新再看；一直这样就看主服务日志 |
| 未知：「还没有成功的备份，离逾期线还有 …」 | 刚开始服务，还没成功过，也还没到逾期线 | 等；到了线还没成功会变成逾期 |

封存正文过了保留期之后，worker 下一步把作业记成 `source_expired` 失败（不重传），健康页会从「卡住」变成
「最近收口的作业没成功」，主服务按批准的间隔再封存一次。worker 没在跑时它不会记任何结果，这时健康页先显示
卡住或逾期，再到服务管理器里看它的状态。

## 仓库维护前先停 worker（维护围栏）

抽查带 `--no-lock`（restic 拿锁时不跳过陈旧锁，被强制结束的检查留下的独占锁会挡住之后所有备份）。所以用管理员
身份做 `prune`、`repair`、`unlock` 之前，先 `systemctl stop studiolo-backup-worker.service studiolo-backup-schedule.timer`，
做完再起；不然抽查可能和维护撞上而误报。误报在那一组重查干净后消掉，但健康页会降级一阵。

健康页「备份仓库检查」一节降级时：

| 健康页显示 | 什么意思 | 先查什么 |
|---|---|---|
| 「第 n 组 … 读到 N 个损坏的数据包，要用管理员身份按维护步骤修」 | restic 报了损坏的数据包 | 先停 worker，用管理员身份 `restic check --read-data-subset n/30` 复核，再按 restic 的修复文档处理；修好后起 worker，等那一组重查干净 |
| 「第 n 组 … 检查没通过（报错 N 处），可能是仓库损坏，也可能是连不上或凭据不对」 | 有错误、没报损坏的数据包 | 先看是不是连不上仓库或凭据过期（备份作业那一节是不是也在失败），再按上一行复核 |
| 「最近一次干净的检查是 … 前，超过 72 小时」 | 一直没查成 | worker 在不在跑、日志里是不是反复「仓库抽查没派下来」或「让给了备份作业」 |
| 「首轮之后每组 32 天内都要读一遍：第 … 组超过 32 天没读干净」 | 有组一直没读干净 | 同上，以及那几组的最近结果 |

## 出事怎么停

停掉并禁用定时器和 worker 单元即可，主服务不受影响：没有 worker 来连，主服务不封存、不上传，只是健康页过一阵显示逾期。
要连监听也停掉，人侧启动器得去掉 `--backup-config` 并重启（`backup.json` 可以留着，不给这个参数就不启用）。
**生产 169 上不要就地改启动器**：发行目录里的 `launch-human.py` 由 `manifest.json` 逐字节钉住，改了之后下一次出候选和激活
都会在核验上一发行时被拒。169 上停 worker 和定时器就够；非要停监听，走一次去掉这个参数的新发行，或者就地改完、
在出下一个候选之前把字节原样改回（改之前留一份原件，**放在发行目录以外**：发行目录里多出一个文件，核验上一发行时
同样会被拒；改回后对 `manifest.json` 里的摘要）。
r17 起人侧单元把 `/run/studiolo-backup`、`/var/lib/studiolo-backup-staging` 写成必须存在的写根：停用备份时
tmpfiles 片段和这两个目录都要留着，启动器不带 `--backup-config` 也一样，删了人侧单元就起不来；要彻底撤掉，
走一次去掉这两行的新发行。
不要去改 owner.json：备份两段本来就不在里面，改它的字节会让 Bridge 绑定对不上、人侧起不来。
已经上传的快照、worker 状态目录和账本里的记录都不删。
