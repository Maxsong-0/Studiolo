# 人侧被强杀或崩溃之后怎么恢复

适用：`studiolo-human.service` 被强杀（停止超时到点、内存超限、`kill -9`）或崩溃之后起不来；
或者激活、回滚脚本停下，回执里写着 `program_recovery: blocked_requires_operator`。
在生产机上做，要 sudo 的命令由主人本人敲。整机重启、断电之后 `/run` 是空的，第 1～3 节说的残留节点不会有；
第 4 节的暂存区在 `/var/lib` 下，重启之后东西还在，照样适用。

r27 起的人侧发行（T-231、BUG-058）会自己接管死 socket（r26 只做到一半：程序会接管，启动器见到 `board.sock` 还是拒绝，
强杀之后要人删 `board.sock` 一个再起；r25 和更早的都不会）：程序握着主库锁、连一下没人监听、
节点类型属主链接数都对、删之前再核还是同一个，才删那一个，日志里有「接管上一任留下的…入口 socket」。所以强杀之后一般几秒内自己起来，
先看第 1 节的日志：有这几行、人侧是 `active`，就不用往下做，只在下一次激活之前清一下重启计数（第 2 节末尾）。
2026-11-24 在 169 上真强杀过一次（没有封存、没有同步在跑的空当）：3.9 秒后自己起来，三个入口都是程序接管的。
自己起不来的还有两种极窄的情形和 socket 无关（另有一种起得来但同步一直停），见第 1 节末尾，不要按残留节点处理。
它判不准（节点不是预期的样子、有人在监听、主库锁在别人手里）照旧拒绝启动，那时、以及 r26 和更早的发行、以及激活或回滚脚本停下的时候，按下面做。

人工恢复就是三件事：看清留下了什么 → 确认它确实是死的 → 只删那一个节点。
看不清就停下，不删。数据库和它旁边的 `-wal`、`-shm`、Bridge 的状态文件、`.producer-lease` 都不要动：下次启动自己接着用。

依据的代码（改了它们要回来对这份步骤）：

- 上一发行现装的启动器和人侧单元原文：`deploy/blackboard/activated/` 下的 `launch-human.py`、`studiolo-human.service`
  （重试几次、`UMask`），和装配钉着的那一处核对 `deploy/blackboard/assembly.py` 的 `INSPECT_LEFTOVER`（r27 换进去的，之后原样沿用；`board.sock`、`http.sock` 不在就过；
  自己的、只有一个名字的 socket 放过去交给程序判；别的拒绝）；
- `crates/local-server/src/unix_ingress/leftover.rs`（判死和接管，三个入口共用）、`crates/local-server/src/coordinator/sole.rs`
  （握着主库锁的凭据）、`crates/local-server/src/unix_ingress/layout.rs` 的 `revalidate` 和 `vacant`、
  `crates/local-server/src/backup/listener.rs`（接管之后还有同名节点一律拒绝；先建节点、再改属组和权限）、`crates/local-server/src/startup/serving.rs`（三个监听谁先谁后）、
  同目录的 `error.rs`（启动报错的原文）；
- `crates/local-server/src/backup/producer.rs`（回收和封存的日志原文、回收失败不挡封存）、
  `crates/local-server/src/health/backup.rs`（健康页的说法）；
- `deploy/blackboard/activation_ops.py` 的 `stopped` 和 `backup_ready`；
- `crates/sync-transport/src/store/publish.rs` 的 `open_with_residue`、`discard_residue` 和 `owned_run/owner.rs` 的 `open_existing`
  （`state.json.publish` 核过权威记录和审批才丢；r27 及更早是见到就拒绝）、
  `crates/local-server/src/sync_plane/driver/headless.rs` 的 `settle_active`（停在 `Prepared` 的运行收成「没执行」）和同目录 `diagnostics.rs`（日志原文）、
  `crates/vault-broker/src/mutation/run.rs`、`writer.rs` 的 `write_status`、`crates/local-server/src/coordinator.rs`
  （保存停在 `Prepared` 或 `Installing` 就起不来）、`crates/local-server/src/maintenance/`（离线命令）；
- `crates/backup/src/checkpoint/staging/` 下的 `creation.rs`（建目录和登记谁先谁后）、`lifecycle.rs`（`begin` 见到
  `removing-` 就不开始，`finish` 删登记目录）、`housekeeping.rs`、`housekeeping/plan.rs`（回收的先后、哪种残缺报哪个错、
  怎么认删到一半的 `removing-`）、`io/cleanup.rs` 的 `retire`（登记目录先改名、`identity.json` 最后删）、
  `journal.rs` 的 `removing_name`（`removing-` 名字的写法）、`budget.rs`（每一步删除之前先看期限）、
  `io.rs` 的 `capacity`（剩余空间下限）。

第 1～3 节的命令没有在真机上演练过整套；第 4 节的手工修法是从代码的不变量推出来的，没有现成工具，也没有演练过。

## 1. 先看现象（只读）

```bash
systemctl show studiolo-human.service studiolo-agent.service -p ActiveState -p SubState -p MainPID -p NRestarts -p Result
```

```bash
journalctl -u studiolo-human.service -n 40 --no-pager
```

```bash
ls -la /run/studiolo-human /run/studiolo-board /run/studiolo-backup
```

| 日志里的话 | 挡着启动的节点 |
|---|---|
| `studiolo-human: startup rejected` | 启动器拒绝。r27 起：`/run/studiolo-human/http.sock`、`/run/studiolo-board/board.sock` 里有一个在、而且不是自己的、只有一个名字的 socket。r26：`board.sock` 在就拒绝，`http.sock` 同 r27。r25 和更早：两个里有一个在就拒绝。启动器别的前置检查没过也是这句：先看这两个节点在不在 |
| `可信Unix入口配置或监听失败；不覆盖既有路径` | 程序拒绝。三个里的任一个判不准（`http.sock` r26 起、`board.sock` r27 起才走得到这里）；`/run/studiolo-backup/producer.sock` 启动器不看，一直是这句 |

单元是 `Restart=on-failure`、隔 3 秒再试、60 秒内最多 3 次：有节点挡着的话，它试满 3 次就停在 `failed`，不会一直转。

日志是下面两句之一、而且刚被强杀或断过电，不是残留节点，第 2、3 节都不管用：什么都不要删，把第 1 节三条命令的输出交给开发会话。
两种都是程序有意失败关闭（不确定就不开），窗口很窄（毫秒级，写盘慢时更宽），T-231 之前就有，都没有在真机上演练过：

- `双向 Bridge 平面装配失败；审批文档、受控 launcher 或私有存储未就绪，未监听`：可能正好杀在传输库换状态的那一下（或那一刻写盘报错，比如盘满），
  传输私有目录里留下了 `state.json.publish`。**r27 及更早**：程序一律不认领，没有现成的处理命令。
  **r28 起**（带上 T-239；r28 激活之前 169 上还没有）：启动时核过权威记录和审批就自己丢，日志一行
  `studiolo_bridge component=transport_store outcome=publish_residue_discarded`，不用人管；
  残留的形态不对（不是本人、0600、单链接、大小合规的普通文件），或者 `state.json` 缺、坏、和审批对不上，照旧拒绝、等人看，两个文件都不动。
- `既有canonical写者或完整监视视图不可用；不初始化、不修复、不回退`：可能正好杀在一次单篇保存落盘的中间，
  写者记录停在 `Prepared`（准备好、还没装）或 `Installing`（正在装、没有结局），停在哪一步都起不来。
  这一种有离线命令（`docs/m2/writer-maintenance.md`）：人侧停着时 `studiolo-maintain inspect` 看停在哪一步，
  `Prepared` 用 `cancel-prepared`，`Installing` 用 `preserve-installing`。没在 169 上跑过（要真终端、要锁定内存），
  由开发会话带着做。文件操作停在半路也用这两条（169 没开文件操作）；批量保存下次启动会接着做完，
  停机期间那篇文件又被外面改过的除外（没有离线命令）。

这两句也会因为别的原因出现（配置、目录、权限不对），所以先交人看，不先认定是它。

还有一种不挡启动：人侧起来了，但同步每轮都失败（日志每轮一行 `studiolo_bridge step=Inspect error=Transport`）、封存也跟着失败。
可能正好杀在登记了一次同步运行、还没发启动的那一下，传输记录停在 `Prepared`。**r27 及更早**：没有入口能收掉它，重启机器也不解，
什么都不要删，把日志交给开发会话。**r28 起**（带上 T-239；r28 激活之前 169 上还没有）：下一轮同步自己收成「没执行」，
日志一行 `studiolo_bridge component=transport outcome=unlaunched_run_cancelled`，再跑完一次同步封存就恢复。没有在真机上演练过。
同步失败也会因为 launcher 等别的原因出现，同样先交人看。

## 2. 确认节点是死的，再删

对每一个还在的节点，下面四条都成立才删；有一条不成立就停下。r26 上只有 `board.sock` 要删：删了它再起，`http.sock`、`producer.sock`
由程序自己接管（2026-11-24 在 169 上这样恢复过）；r27 起程序三个都自己接管，走到这里说明有一个它判不准。

1. 没有进程在用它（监听着的、刚绑上还没监听的、连着的都算）。把节点的路径填进去，这条**没有输出**才算（有输出说明有活的进程在用，它不是残留）：

   ```bash
   ss -xa | grep -F /run/studiolo-human/http.sock
   ```

2. `http.sock`、`board.sock` 还要人侧停着（只有人侧起不来时才需要删它们）：第 1 节第一条命令里，人侧是
   `ActiveState=inactive` 或 `failed`，`MainPID=0`。还在反复重试（`activating`）就先停住它：
   `sudo systemctl stop studiolo-human.service`，再看一遍。
   `producer.sock` 看人侧现在的状态：人侧没在跑（带备份的新发行自己起不来，挡着它的可能正是这个节点），和另外两个一样，
   先确认它停住了；人侧在跑（`active`）而第 1 条没有输出，说明这个节点没人用（回滚到不带备份的旧发行之后就是这样），
   不用停人侧。起点和终点都带备份的发行（r17→r18 起）不会出现后一种：带备份的人侧一启动就自己监听这个节点，
   节点留着它就起不来，和另外两个一样处理。
3. 节点是预期的样子：类型是 socket，属主 maxsong，链接数 1；`http.sock`、`board.sock` 权限 600，
   `producer.sock` 权限 660、属组 `studiolo-backup-read`。把还在的那个路径填进去：

   ```bash
   stat -c '%F %U:%G %a 链接数=%h %n' /run/studiolo-human/http.sock
   ```

   类型、属主、链接数三样都对，只有权限或属组不是上面的值：这是程序刚建好节点、还没来得及改权限就被杀
   （按单元的 `UMask=0077` 推，这时权限是 700，属组是 maxsong 自己的组），照样算残留，往下做。
   三样里有一样不对就停下，把这条命令的输出交给开发会话。
4. 用 maxsong 账号删这一个节点。不带 `-r`，不删目录（三个目录都要留着）：

   ```bash
   rm -- /run/studiolo-human/http.sock
   ```

删完再跑一遍第 1 节第一条命令。不管删的是哪一个节点，人侧是 `failed` 或 `inactive`（`MainPID=0`）就把它起回来；
人侧本来就在跑的（回滚之后只删 `producer.sock`）不用起。先清掉「试太多次」的记录，不然 systemd 不肯再起：

```bash
sudo systemctl reset-failed studiolo-human.service
```

```bash
sudo systemctl start studiolo-human.service
```

人侧自己起回来的（没动过节点）也要跑上面那条 `reset-failed`，但不用 `start`：激活脚本要求 `NRestarts=0`
（`deploy/human/release/runtime_checks.py`），systemctl(1) 写明 `reset-failed` 会把它清零，正在跑的服务不受影响。

再跑一遍第 1 节第一条命令：人侧应是 `ActiveState=active`、`NRestarts=0`。socket 用第 1 节第三条命令看：
起来的是带备份的新发行就有三个；是旧发行（第 3 节 `stop-old` 那种情形）就只有 `http.sock`、`board.sock` 两个。
网页和健康页能开。
agent 服务（`studiolo-agent.service`）要是也停了，人侧起来之后再用同样两条命令起它。

## 3. 激活或回滚脚本停在 `blocked_requires_operator`

脚本最后打出的那行回执里有 `stage` 和 `deployment_record`（记录目录，root 私有）。先看两样：

```bash
sha256sum /etc/systemd/system/studiolo-human.service /etc/systemd/system/studiolo-agent.service
```

再用 sudo 读记录目录里的 `preflight.json`（把回执里的目录填进去），里面的 `old_units`、`new_units` 是换之前、换之后两个单元的摘要。

- `stage` 是 `stop-old`，两个单元的摘要都等于 `old_units`：旧程序停服时被强杀，新单元还没装，数据库没动。
  停服之后的核对看到 `http.sock`、`board.sock` 或 `producer.sock`（r18 的激活脚本起核这一个：上一发行 r17 已在监听它，
  正常关停会删掉）还在就拒绝，回滚再停一次也过不了同一条核对，所以两个服务都停着。
  照第 2 节删节点、起人侧，再起 agent：现场就回到激活之前。要不要重试激活另外定；重试时预检会把全部前提再核一遍。
- 别的 `stage`，或者单元的摘要不是 `old_units` 那一对：不要手工换单元，也不要手工启动。
  把回执、`preflight.json`、记录目录里的 `failed.json` 和上面两个摘要交给开发会话判断。
- 回滚成功了（旧程序在跑），只留下 `producer.sock`（只在回滚到不带备份的发行、r16 及更早时才有这种现场；
  从 r18 回滚到 r17 时它留着的话停服后的核对就先拒绝了，见上一条）：它不挡回滚，挡的是**下一次**激活的预检。
  照第 2 节的第 1、3、4 条删掉它（不用停人侧），再重试激活。
- 回滚成功了，换回来的旧程序带备份、又是提交 `a2174fd` 之前构建的，换下去的是 `a2174fd` 起的新版：
  看一眼 `ls /var/lib/studiolo-backup-staging`，有 `removing-` 开头的目录就照第 4 节「新版回滚到旧版」处理，
  不然旧版的回收每一轮都过不去、到期的副本收不走。回滚到不带备份的发行（r16 及更早）不受影响。
  按现在的发行这一条遇不到：第一个带备份的发行 r17 是提交 `0784ae9` 构建的，已含 `a2174fd`，从它回滚只会回到 r16
  （`2df75ca` 构建的那份 r17 候选没激活过，2026-11-21 重出时撤下了）。留着是防以后有人把 `a2174fd` 之前的带备份构建装上去。

## 4. 封存或回收做到一半被杀（备份暂存区）

多数情况不用管：人侧下次启动的第一轮回收会把登记过的半成品收走；没做完的那一次，下一次定时备份会补一条失败记录，
健康页「备份作业」一节的结论显示「最近收口的第 N 号作业没成功：冻结、导出或准入没做成（跨 owner 屏障没有建立）；
按批准的间隔重试」。有两句排在它前面，先看到的可能是它们：人侧停得比保留期（现在批准的是 1 小时）还久，重新起来之后、
下一次定时备份之前，结论是「第 N 号作业挂了……还没收口，超过保留期……，算卡住」；超过两倍间隔没有成功过，结论是
「……算逾期」。这两种时候，失败的原因在点开「查看备份作业」之后的「最近失败」一行里。
失败不缩短间隔，等满批准的间隔、下一次备份成功之后恢复正常。

要人处理的是回收自己过不去的时候：日志里每一轮都是 `备份暂存回收未完成：CleanupRequired` 或 `…：Changed`。
人侧照常启动、照常服务。按封存还成不成分两种样子；回收报的是哪一个词，每种里常见的写在前面，另一个词也会有，一样照下面看：

- 封存也每次失败（`备份封存未完成：BarrierNotEstablished`），回收常见的是 `CleanupRequired`：备份一直失败。
  这是挡着回收的那样东西也挡着新的封存（`incomplete-`、`export-`、`pending-` 开头的残留，新版的 `removing-`）；
  这类残留里面有对不上的东西时回收报的是 `Changed`，封存照样每次失败。
  健康页上「冻结、导出或准入没做成（跨 owner 屏障没有建立）」这几个字起先在「备份作业」一节的结论里；
  一直失败超过两倍间隔之后结论改成「……算逾期」，这几个字只剩在「查看备份作业」的「最近失败」一行里。
- 封存照常成功、健康页也正常，回收常见的是每一轮 `Changed`：到期的副本不再被收走，暂存区一直涨
  （正常只留保留期之内的几份 `retained-`：现在批准的是留 1 小时、10 分钟一份，6～7 份），涨到所在的盘剩不到 2 GiB，
  封存开始失败，健康页显示「磁盘空间不够」（位置同上一条：起先在结论里，逾期之后只在「最近失败」一行）。
  这是挡着回收的东西不挡封存（不完整的 `retained-`）；回收每一轮报 `CleanupRequired` 而封存照常的，是暂存区里有
  登记里对不上号的名字（没有登记的 `sealed-`、回收不认识的名字）：从新版回滚到旧版之后留下的 `removing-` 就是这种，
  见表后「新版回滚到旧版」。
  健康页正常的这段时间只有日志和暂存区的大小看得出来：

```bash
journalctl -u studiolo-human.service --since "2 hours ago" --no-pager | grep -F 备份暂存回收未完成
```

```bash
du -sh /var/lib/studiolo-backup-staging
```

```bash
df -h /var/lib/studiolo-backup-staging
```

原因都是暂存区里有一样东西对不上登记。先停掉定时器和 worker，
等正在跑的那一次定时请求结束（第二条命令输出 `inactive` 或 `failed` 才往下），再看：

```bash
sudo systemctl stop studiolo-backup-schedule.timer studiolo-backup-worker.service
```

```bash
systemctl is-active studiolo-backup-schedule.service
```

```bash
ls -la --time-style=full-iso /var/lib/studiolo-backup-staging
```

```bash
systemctl show studiolo-human.service -p ActiveEnterTimestamp
```

人侧还在跑，所以先确认要动的不是它**正在做**的那一次封存，两条都成立才算残留：日志里已经出现过
`备份暂存回收未完成：CleanupRequired` 或 `…：Changed`（回收和封存拿同一把锁，回收读到了、拒绝了，说明那一刻没有封存在做）；
要动的目录的时间早于人侧这次启动的时间（上面最后一条）。拿不准就先把人侧停下再看
（`sudo systemctl stop studiolo-human.service`，最长等 300 秒），修完照第 2 节末尾起回来。
目录的时间晚于这次启动也按拿不准办：旧版回收超时或出错正好停在删登记文件中间，留下的东西和被杀一样（新版、旧版见表后）。

目录名是 `前缀-<64 位十六进制>`，同一次封存的几个目录后面那一串相同；`incomplete-`、`retained-` 是登记目录，里面只放
登记文件。新版还有第三种登记目录 `removing-<K>-<一串数字>`：程序删登记目录之前先把它原子改成这个名字
（数字是这个目录自己的 inode 号，`ls -lid` 打出来的第一列），再删里面的登记文件，`identity.json` 留到最后，最后删目录。
不管停在哪一步（期限到、出错、被杀），下一轮回收都按名字里的 inode 认出它、接着删完，日志里不报错。它还在的时候
新的封存不开始；定时请求总是先回收、再封存，所以通常同一次请求里回收先把它删完、封存照常，回收这一轮又没删完时
这一次封存记一条失败，下一轮接着删。**新版在跑的时候它不用人管，不要手工删**；人侧从新版换回了旧版之后还留着的，
见表后「新版回滚到旧版」。

下表前两行是「刚建好目录、登记还没落盘就被杀」，按代码的顺序它们正常是空的，所以只用 `rmdir`
（目录不空它会拒绝，那就停下）。后两行是登记目录自己不完整：封存刚开头、`identity.json` 还没落盘就被杀
（里面最多一个 `identity.new`，新版旧版都会有）；或者旧版回收删登记目录删到一半就停了（旧版按名字顺序一个个删，
`export.json`、`identity.json` 先没，排在后面的还在），只有旧版会留下。
登记文件用 `rm --` 一个个删，不带 `-r`，删完再 `rmdir`：

| 看到的 | 还要成立 | 怎么修 |
|---|---|---|
| `export-<K>` 在，`incomplete-<K>/` 里没有 `export.json` | `ls -la` 两个目录看过 | `rmdir` 这个 `export-<K>` |
| `pending-<K>` 在，`incomplete-<K>/` 里没有 `pending.json` | 同上 | `rmdir` 这个 `pending-<K>` |
| `incomplete-<K>/` 里没有 `identity.json` | 同一个 `<K>` 的 `export-`、`pending-`、`sealed-` 都不在；里面剩下的都是普通文件，名字只在 `identity.new`、`pending.json`、`pending.new`、`sealed.json`、`sealed.new` 这五个里（空目录也算） | 删掉这几个文件，再 `rmdir` 这个 `incomplete-<K>` |
| `retained-<K>/` 里不满四份（齐全是 `export.json`、`identity.json`、`pending.json`、`sealed.json`） | 日志是 `Changed`；`sealed-<K>` 不在；worker 停着；里面剩下的都是普通文件，名字只在 `identity.json`、`pending.json`、`sealed.json` 这三个里（空目录也算） | 删掉这几个文件，再 `rmdir` 这个 `retained-<K>` |

新版、旧版：「回收删登记目录删到一半之后自己续不上」这个缺陷 2026-11-21 独立复查发现，同日在 main 上修好
（T-216 F16，提交 `a2174fd`）。带这个修法的构建是新版，之前的带备份构建是旧版。169 上第一个带备份的人侧发行 r17 是提交 `0784ae9`
构建的，已含这个修法（`2df75ca` 构建的那份 r17 候选没激活过，2026-11-21 重出时撤下了）：生产上的备份生产者从第一天起就是新版，
旧版没上过生产，本节说到旧版的地方只在以后有人把旧版构建装上去跑过才用得上。
新版：删到一半停下的由下一轮回收接着删完，开头那两种样子不会再因为这个缺陷出现；
还会碰到的只有前两行和第三行里「最多一个 `identity.new`」那种（都是建目录和写登记之间被杀），照表删。
旧版跑过的现场：开头那两种样子照旧会因为这个缺陷出现，照后两行手工删；之后换上新版，旧版已经留下的半截登记目录
新版回收照样认不出（不按名字猜着删），还是 `CleanupRequired` 或 `Changed`，照后两行手工删一次就不再有。
旧版另有一种先别动：封存把登记改名成 `retained-<K>` 之后、交出引用之前出错，回滚先删了 `sealed-<K>`、没删完登记，
留下四份齐全的 `retained-<K>` 而 `sealed-<K>` 不在，日志每轮 `Changed`；到期（保留期，现在批准的是 1 小时）之后
回收自己收走，不用手工删，过了保留期还在才交开发会话。新版回滚先把它改回 `incomplete-<K>` 再删，不会再这样。

新版回滚到旧版：`removing-<K>-<数字>` 这个名字只有新版认得。新版留下了它（删登记目录时在改名之后、删完之前停下：
期限到、出错、被杀），下一轮还没删完人侧就换回了旧版（提交 `a2174fd` 之前构建的带备份发行；按现在的发行没有这样的，见上一段），
旧版的回收见到不认识的名字，每一轮都整轮拒绝（`CleanupRequired`），封存照常做：就是开头第二种样子，到期的副本一份都收不走。
所以打算从新版回滚之前，先看暂存区里有没有 `removing-`，有就等新版的下一轮回收（定时器每 5 分钟问一次）把它删完再回滚。
已经回滚了才看到：换回新版让它自己删完最省事。不换回，就照前面停掉定时器和 worker、确认它是残留，下面三条都成立才手工删，
有一条不成立就交开发会话：名字是 `removing-<64 位十六进制>-<十进制数字>`，数字等于 `ls -lid` 打出的第一列；
同一个 `<K>` 的 `export-`、`pending-`、`sealed-`、`incomplete-`、`retained-` 都不在；里面剩下的都是普通文件，名字只在
`identity.json`、`identity.new`、`export.json`、`export.new`、`pending.json`、`pending.new`、`sealed.json`、`sealed.new`
这八个里（空目录也算）。用 `rm --` 一个个删，`identity.json` 留到最后删，再 `rmdir` 这个目录。

别的情况都不要自己删，把 `ls -laR /var/lib/studiolo-backup-staging` 的输出交给开发会话：
没有 `identity.json` 的 `incomplete-<K>` 旁边还有同一个 `<K>` 的目录；登记目录里有表里没列的名字，或者不是普通文件；
名字不是上面这几种前缀的东西；四份齐全的 `retained-<K>`（这是做完的封存，回收正常时到期会被收走，worker 可能正要读它；
日志是 `Changed` 而每个 `retained-` 都四份齐全，也交开发会话，只有 `sealed-<K>` 不在的那种先照上面「新版、旧版」一段等过保留期）；新版在跑、却一直不消失的 `removing-<K>-<数字>`（每一轮日志都是
`CleanupRequired` 或 `Changed`，说明里面有程序没留下过的东西：名字不是程序写的那种、数字和 `ls -lid` 对不上、
`identity.json` 没了还剩别的文件、有表外的名字，或者同一个 `<K>` 旁边还有别的目录）。`.producer-lease` 任何时候都不删。

修完把两个单元起回来，等下一次定时备份（定时器每 5 分钟问一次，批准的间隔是 10 分钟）：

```bash
sudo systemctl start studiolo-backup-worker.service studiolo-backup-schedule.timer
```

健康页「备份作业」一节的结论应回到「最近一次成功是……前的第 N 号」；日志里还是 `CleanupRequired` 或 `Changed` 就停下，不要再删。
第二种样子（封存照常、暂存区一直涨）修好之后，积压的到期副本由之后的几轮回收收走（一轮最多做 30 秒），`du` 看到的大小应该降回去。
暂存区里只有导出的副本，原始数据不在这里：删错的代价是这一次备份重做，不会丢库里的内容。
