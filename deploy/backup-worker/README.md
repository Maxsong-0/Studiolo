# 独立备份 worker 的部署模板

T-204。本目录是候选发布源，**尚未安装到任何机器**。专用账号、读取组、目录、固定 restic 包和仓库凭据
都要用户本人建好并逐项验收；测试箱上的结果不能代替这一步。契约见
[`docs/contracts/backup-producer.md`](../../docs/contracts/backup-producer.md) 和
[`docs/contracts/backup-worker-process.md`](../../docs/contracts/backup-worker-process.md)。

## 三个角色

| 角色 | 账号 | 做什么 | 拿不到什么 |
|---|---|---|---|
| 主服务（生产者） | 人侧服务原来的账号，加进读取组 | 发序号、冻结并封存、写准入、答队列 | 仓库凭据 |
| worker（`studiolo-backup-worker run`） | 专用账号 `studiolo-backup`，加进读取组 | 取待办、读封存、追加上传、核对、写结果 | 账本文件、任何 owner 的库 |
| 定时触发（`… schedule`） | 同 worker 账号 | 每五分钟请主服务判断一次该不该封存 | 凭据、正文 |

读取组（示例名 `studiolo-backup-read`）只用来让 worker 读最终封存和连备份 socket。

## 程序的三个子命令

没有别的参数，也不读环境变量：

- `run`：常驻。启动先收紧秘密驻留面、读 `/etc/studiolo/backup-worker.json`、打开状态目录，把上次没做完的阶段标成
  未知；之后每 30 秒问主服务「下一个待办」，对它做一步。子进程清理没能证实时退出，由 `Restart=on-failure` 拉起。
- `schedule`：一次性。输出 `started`、`outstanding` 或 `not_due` 之一，三种都算成功；连不上主服务才失败。
- `init`：显式初始化 `/var/lib/studiolo-backup-worker/state`。已经有锁或状态就拒绝，不会重置序号。

## 安装前要有的东西

1. 专用账号 `studiolo-backup` 和读取组；主服务账号和 worker 账号都在读取组里。
2. `/usr/lib/studiolo-backup-worker/`（root 所有、不可组写）里放：
   - `studiolo-backup-worker`：在 108 用 `cargo --locked build --release -p studiolo-backup-worker` 构建；
   - `restic-0.19.1`：官方发布包里的可执行文件，摘要写进配置的 `restic_sha256`。本仓库不下载它。
3. `/etc/studiolo/backup-worker.json`（root 所有，0644 或 0444），字段见进程边界契约；`producer_socket` 和
   `sealed_parent` 要和主服务私有配置 `backup_producer` 段里的 `socket`、`staging_parent` 是同一组路径，
   `restic_sha256` 和 `restic_digest` 是同一个值。
4. `systemd/studiolo-backup.tmpfiles.conf` 里的两个目录（属主主服务账号、属组读取组、0750）。主服务不建它们。
5. `/etc/studiolo/backup-worker-credentials/`（root 所有，0700）里三个文件（0600）：`repository-password`、
   `rest-username`、`rest-password`。由用户经安全输入写入，不从聊天或日志里复制；单元用 `LoadCredential` 交给
   worker，worker 只从 `/run/credentials/studiolo-backup-worker.service` 读，所以**服务单元名不能改**。
6. 主服务私有配置里加 `backup` 和 `backup_producer` 两段（成对）。主服务单元（`deploy/human/studiolo-human.service`）
   同时改两处，再重启主服务：
   - 单元是 `ProtectSystem=strict`：给备份 socket 的父目录和封存父目录各加一行 `ReadWritePaths=`（路径和第 4 步
     tmpfiles 片段里的一致）。不加的话绑不上 socket、建不了封存，配了生产者的主服务会拒绝启动。
   - 停止超时：关停时要等在途的回收（最多 30 秒）和封存（最多一个冻结预算，上限 120 秒）做完。单元里现在的
     `TimeoutStopSec=90` 不够，按「冻结预算 + 60 秒」以上配。超时被强杀时 socket 节点会留下，下次启动按设计
     拒绝（不清理也不接管），要人看过再删。
7. **上线前在 Mac 上做一次只读核查**（2026-11-16 用户拍板），有任何一项不为空就先别启用备份，报告用户再定：
   - Vault 根下 `sources/attempts/<任务>/<尝试>/` 里有没有第 1 版清单或者没有清单的尝试目录
     （`find <Vault 根>/sources/attempts -mindepth 3 -maxdepth 3 -name manifest.json -exec grep -L '"format_version": *2' {} +`
     列出不是第 2 版的清单；再对照 `find … -mindepth 2 -maxdepth 2 -type d` 看有没有缺清单的目录）。这类目录会让
     每一次整批封存都失败，而且因为历史版本还在，手工删不掉。
   - 写者数据目录里有没有 `artifact-candidate-*.json`（还没发布的暂存正文；升级前留下的没有原批准，同样让封存失败）。
   - 控制库里有没有停在「提交中」的 attempt（只读打开 `control.sqlite3`，`SELECT task_id, attempt_id FROM attempts
     WHERE phase = 'committing'`）。有的话控制库检查点拒绝导出；升级前留下的旧内联预留还会让启动和周期对账整轮失败。

## 验收顺序

1. `systemd-analyze verify` 三个单元；核对目录和文件的属主、属组、权限。
2. 以 worker 账号跑一次 `studiolo-backup-worker init`，再跑一次应失败（已经初始化）。
3. 手动跑一次 `studiolo-backup-worker schedule`：第一次应输出 `started`，紧接着再跑应输出 `outstanding`。
4. 启动 `studiolo-backup-worker.service`，日志里应出现「备份作业已收口：Succeeded」，人侧账本里这个原键有一条
   成功结果。**主服务健康页的备份一栏还没接线**（只读投影 `BackupEvidence` 没接进组合根，另立任务），这一步
   不看健康页。
5. 停掉 worker 再触发一次封存，过了保留期再启动：应记一条 `source_expired` 失败，不重传。
6. 用恢复演练（另一台机器、另一套恢复凭据）读回这一次的快照，对照封存清单。新根恢复现在整次拒绝（见
   `docs/contracts/new-root-restore.md`），这一步只做到读回并核对为止。

每一步的输出留证；第 4～6 步要真实仓库，测试箱做不了。

## 现在怎么发现作业卡住了

健康页接线之前，只有这几种信号：`schedule` 一直输出 `outstanding`；worker 日志反复出现「备份 worker 等下一轮」
或「备份作业这一步被拒绝」；封存正文过了保留期之后，worker 下一步把作业记成 `source_expired` 失败（不重传），
主服务按批准的间隔再封存一次。worker 没在跑时什么都不会记，要看服务管理器里它的状态。封存失败之后要等满
批准的间隔才重试，失败不缩短间隔（2026-11-16 用户拍板）。

## 出事怎么停

停掉并禁用定时器和 worker 单元即可，主服务不受影响；主服务私有配置里去掉那两段并重启，备份 socket 就不再
监听。已经上传的快照、worker 状态目录和账本里的记录都不删。
