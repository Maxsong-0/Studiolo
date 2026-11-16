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
7. **上线前在 Mac 上做一次只读核查**（2026-11-16 用户拍板）：脚本经 ssh 送到存放数据的机器上，以主服务账号跑，
   退出码不是 0 就先别启用备份，把输出报告用户再定：

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
   - 封存收不了的节点：符号链接、设备、套接字、管道，多个硬链接的文件，别的账号的文件，挂载点，
     不是 UTF-8 或带控制字符的名字，导出扫描认不了的路径（编码过的分隔符、非 NFC 拼写等），层数和文件数超上限。
   - 控制库：停在「提交中」的 attempt，旧格式的终态（升级前的旧内联预留、报告不是第 3 版），以及检查点校验里
     其余各条（发件箱没投递完的可能是瞬时的，过一分钟再跑一次）。旧内联预留现在只挡它自己那个 attempt 的对账
     （2026-11-16），但控制库检查点仍然拒绝导出，所以照样挡封存。
   - 没核的（由封存本身把关，第一次封存失败会写明原因）：历史目录对历史正文的认领、暂存正文的内容和名字是否
     对得上、清单里原批准的内部字段、控制库表结构清单和产物批准的逐条核对。

## 验收顺序

1. `systemd-analyze verify` 三个单元；核对目录和文件的属主、属组、权限。
2. 以 worker 账号跑一次 `studiolo-backup-worker init`，再跑一次应失败（已经初始化）。
3. 手动跑一次 `studiolo-backup-worker schedule`：第一次应输出 `started`，紧接着再跑应输出 `outstanding`。
4. 启动 `studiolo-backup-worker.service`，日志里应出现「备份作业已收口：Succeeded」，人侧账本里这个原键有一条
   成功结果。再打开主服务健康页（登录后 `/api/v1/health`，或工作台「本机健康」）：「备份作业」一节应为
   正常，正文是「最近一次成功是 … 前的第 N 号」，N 就是这个原键的序号；展开「查看备份作业」能看到同一个序号
   和成功时刻，「挂着的作业」为「没有」。健康页只说作业，恢复点年龄那一节照旧是未知（恢复点只在 worker
   自己的状态库里，主服务读不到）。
5. 停掉 worker 再触发一次封存，过了保留期再启动：应记一条 `source_expired` 失败，不重传。
6. 用恢复演练（另一台机器、另一套恢复凭据）读回这一次的快照，对照封存清单。新根恢复现在整次拒绝（见
   `docs/contracts/new-root-restore.md`），这一步只做到读回并核对为止。

每一步的输出留证；第 4～6 步要真实仓库，测试箱做不了。

## 现在怎么发现作业卡住了

先看主服务健康页的「备份作业」一节（2026-11-16 接上）。它只读人侧账本里的备份事件，阈值来自私有配置：

| 健康页显示 | 什么意思 | 先查什么 |
|---|---|---|
| 降级：「第 N 号作业挂了 … 还没收口，超过保留期 …，算卡住」 | 有原意图、没有结果的作业挂得比 `source_retention_millis` 还久；展开明细能看到它封存了没有 | 封存过了（「已封存，等 worker 上传」）：看 worker 单元在不在跑、日志里是不是反复「备份 worker 等下一轮」或「备份作业这一步被拒绝」；没封存：看主服务日志里「备份封存未完成」 |
| 降级：「… 超过两倍最小间隔 …，算逾期」 | 离最近一次成功超过两倍 `schedule_interval_millis`；从没成功过的从开始服务和第一次尝试里较早的那个算起 | 定时器在不在跑（worker 没触发 `schedule` 就什么都不会发生），以及最近失败的原因 |
| 降级：「最近收口的第 N 号作业没成功：…」 | 最近一条作业记了失败或未验证；原因是账本里的固定类别（如「磁盘空间不够」「封存前来源变了」） | 按原因处理；失败之后要等满批准的间隔才重试，失败不缩短间隔（2026-11-16 用户拍板） |
| 未知：「备份生产者没有启用」 | 私有配置里没有 `backup` 和 `backup_producer` 两段 | 是不是有意不启用 |
| 未知：「这一次没读到账本里的备份事件」 | 账本没在扫描预算（5 秒）里读完，或本地时钟不可用 | 刷新再看；一直这样就看主服务日志 |
| 未知：「还没有成功的备份，离逾期线还有 …」 | 刚开始服务，还没成功过，也还没到逾期线 | 等；到了线还没成功会变成逾期 |

封存正文过了保留期之后，worker 下一步把作业记成 `source_expired` 失败（不重传），健康页会从「卡住」变成
「最近收口的作业没成功」，主服务按批准的间隔再封存一次。worker 没在跑时它不会记任何结果，这时健康页先显示
卡住或逾期，再到服务管理器里看它的状态。

## 出事怎么停

停掉并禁用定时器和 worker 单元即可，主服务不受影响；主服务私有配置里去掉那两段并重启，备份 socket 就不再
监听。已经上传的快照、worker 状态目录和账本里的记录都不删。
