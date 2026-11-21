# 人侧被强杀或崩溃之后怎么恢复

适用：`studiolo-human.service` 被强杀（停止超时到点、内存超限、`kill -9`）或崩溃之后起不来；
或者激活、回滚脚本停下，回执里写着 `program_recovery: blocked_requires_operator`。
在生产机上做，要 sudo 的命令由主人本人敲。整机重启不算：`/run` 重启后是空的，不会有下面说的残留节点。

程序对上一次留下的东西一律「不清理、不接管」，所以恢复就是三件事：看清留下了什么 → 确认它确实是死的 → 只删那一个节点。
看不清就停下，不删。数据库和它旁边的 `-wal`、`-shm`、Bridge 的状态文件、`.producer-lease` 都不要动：下次启动自己接着用。

依据的代码（改了它们要回来对这份步骤）：启动器 `deploy/human/current-v2/launch-human.py`（已有 `http.sock` 就拒绝）、
`crates/local-server/src/unix_ingress/layout.rs` 的 `revalidate`、`crates/local-server/src/backup/listener.rs`
（已有同名节点一律拒绝）、`deploy/blackboard/activation_ops.py` 的 `stopped` 和 `backup_ready`、
`crates/backup/src/checkpoint/staging/housekeeping/plan.rs` 和同目录的 `lifecycle.rs`。
第 1～3 节的命令没有在真机上演练过整套；第 4 节的手工修法是从代码的不变量推出来的，没有现成工具。

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
| `studiolo-human: startup rejected` | `/run/studiolo-human/http.sock`（启动器别的前置检查没过也是这句：先看这个节点在不在） |
| `可信Unix入口配置或监听失败；不覆盖既有路径` | `/run/studiolo-board/board.sock`，或者 `/run/studiolo-backup/producer.sock` |

单元是 `Restart=on-failure`、隔 3 秒再试、60 秒内最多 3 次：有节点挡着的话，它试满 3 次就停在 `failed`，不会一直转。

## 2. 确认节点是死的，再删

对每一个还在的节点，下面四条都成立才删；有一条不成立就停下。

1. 没有进程在监听它。把节点的路径填进去，这条**没有输出**才算（有输出说明有活的进程在用，它不是残留）：

   ```bash
   ss -xl | grep -F /run/studiolo-human/http.sock
   ```

2. `http.sock`、`board.sock` 还要人侧停着（只有人侧起不来时才需要删它们）：第 1 节第一条命令里，人侧是
   `ActiveState=inactive` 或 `failed`，`MainPID=0`。还在反复重试（`activating`）就先停住它：
   `sudo systemctl stop studiolo-human.service`，再看一遍。
   `producer.sock` 不要求人侧停着：回滚到不带备份的旧发行之后，旧人侧在跑、这个节点却留着，第 1 条没有输出就是没人用它。
3. 节点是预期的样子：类型是 socket，属主 maxsong，链接数 1；`http.sock`、`board.sock` 权限 600，
   `producer.sock` 权限 660、属组 `studiolo-backup-read`。把还在的那个路径填进去：

   ```bash
   stat -c '%F %U:%G %a 链接数=%h %n' /run/studiolo-human/http.sock
   ```

4. 用 maxsong 账号删这一个节点。不带 `-r`，不删目录（三个目录都要留着）：

   ```bash
   rm -- /run/studiolo-human/http.sock
   ```

删的是 `http.sock` 或 `board.sock` 的话，接着启动人侧。先清掉「试太多次」的记录，不然 systemd 不肯再起：

```bash
sudo systemctl reset-failed studiolo-human.service
```

```bash
sudo systemctl start studiolo-human.service
```

再跑一遍第 1 节第一条命令：人侧应是 `ActiveState=active`、`NRestarts=0`，三个 socket 都回来了；网页和健康页能开。
agent 服务（`studiolo-agent.service`）要是也停了，人侧起来之后再用同样两条命令起它。

## 3. 激活或回滚脚本停在 `blocked_requires_operator`

脚本最后打出的那行回执里有 `stage` 和 `deployment_record`（记录目录，root 私有）。先看两样：

```bash
sha256sum /etc/systemd/system/studiolo-human.service /etc/systemd/system/studiolo-agent.service
```

再用 sudo 读记录目录里的 `preflight.json`（把回执里的目录填进去），里面的 `old_units`、`new_units` 是换之前、换之后两个单元的摘要。

- `stage` 是 `stop-old`，两个单元的摘要都等于 `old_units`：旧程序停服时被强杀，新单元还没装，数据库没动。
  停服之后的核对看到 `http.sock` 或 `board.sock` 还在就拒绝，回滚再停一次也过不了同一条核对，所以两个服务都停着。
  照第 2 节删节点、起人侧，再起 agent：现场就回到激活之前。要不要重试激活另外定；重试时预检会把全部前提再核一遍。
- 别的 `stage`，或者单元的摘要不是 `old_units` 那一对：不要手工换单元，也不要手工启动。
  把回执、`preflight.json`、记录目录里的 `failed.json` 和上面两个摘要交给开发会话判断。
- 回滚成功了（旧程序在跑），只留下 `producer.sock`：它不挡回滚，挡的是**下一次**激活的预检。
  照第 2 节的第 1、3、4 条删掉它（不用停人侧），再重试激活。

## 4. 封存做到一半被杀（备份暂存区）

多数情况不用管：人侧下次启动的第一轮回收会把登记过的半成品收走；没做完的那一次，下一次定时备份会补一条失败记录，
健康页多一条「最近一次失败」，之后恢复正常。

要人处理的是回收自己过不去的时候。日志里是 `备份暂存回收未完成：CleanupRequired`，之后每一次都是
`备份封存未完成：BarrierNotEstablished`，健康页显示「冻结、导出或准入没做成（跨 owner 屏障没有建立）」。
人侧照常启动、照常服务，只是备份一直失败。原因是暂存区里有一样东西对不上登记。先停掉定时器和 worker，
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
`备份暂存回收未完成：CleanupRequired`（回收和封存拿同一把锁，它拒绝过就说明当时没有封存在做）；要动的目录的时间
早于人侧这次启动的时间（上面最后一条）。拿不准就先把人侧停下再看
（`sudo systemctl stop studiolo-human.service`，最长等 300 秒），修完照第 2 节末尾起回来。

目录名是 `前缀-<64 位十六进制>`，同一次封存的几个目录后面那一串相同。下面三种是「刚建好目录、登记还没落盘就被杀」，
按代码的顺序它们一定是空的，所以只用 `rmdir`（目录不空它会拒绝，那就停下）：

| 看到的 | 怎么判断 | 怎么修 |
|---|---|---|
| `export-<K>` 在，`incomplete-<K>/` 里没有 `export.json` | `ls -la` 两个目录 | `rmdir` 这个 `export-<K>` |
| `pending-<K>` 在，`incomplete-<K>/` 里没有 `pending.json` | 同上 | `rmdir` 这个 `pending-<K>` |
| `incomplete-<K>/` 里没有 `identity.json`，同一个 `<K>` 的 `export-`、`pending-`、`sealed-` 都不在 | 同上 | 删掉里面的 `identity.new`（有的话），再 `rmdir` 这个 `incomplete-<K>` |

别的情况都不要自己删，把 `ls -laR /var/lib/studiolo-backup-staging` 的输出交给开发会话：
没有 `identity.json` 的 `incomplete-<K>` 旁边还有同一个 `<K>` 的目录；名字不是上面这几种前缀的东西；`retained-<K>`
（这是做完的封存，到期自己会被收走，worker 可能正要读它）。`.producer-lease` 任何时候都不删。

修完把两个单元起回来，等下一次定时备份（定时器每 5 分钟问一次，批准的间隔是 10 分钟）：

```bash
sudo systemctl start studiolo-backup-worker.service studiolo-backup-schedule.timer
```

健康页「备份」一节最近一次应是成功；还是 `CleanupRequired` 就停下，不要再删。暂存区里只有导出的副本，
原始数据不在这里：删错的代价是这一次备份重做，不会丢库里的内容。
