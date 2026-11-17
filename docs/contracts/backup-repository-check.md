# 备份仓库每日轮流抽查

T-214，2026-11-17。PRD-v2-06 §11.3 要求上线后定期做仓库完整性检查，本文件是这件事的契约。
**没有验证的**：两个系统账号之间的真实连接、真实 REST 仓库、systemd 下常驻循环的一整轮。
官方 restic 0.19.1 的真实 CLI 用例在 108 上显式跑过（见文末）。

相关契约：[restic 命令计划](restic-command-plan.md)、[worker 进程边界](backup-worker-process.md)、
[备份生产者](backup-producer.md)；健康页见 `docs/m2/health-and-alerting.md` §2.2。

## 查什么、怎么查

仓库按数据包 ID 分成 30 组，worker 在两次备份作业之间每次读一组，30 组轮完一圈就把仓库里的数据读过一遍：

```text
restic --json --no-cache --repo <仓库> check --no-lock --read-data-subset <n>/30
```

- n 只能是 1～30（`CheckBucket`）。restic 按数据包 ID 第一个字节对 30 取余分组；结构检查（全部快照的树）
  每次都跑。
- 带 `--no-lock`：`check` 默认加独占锁，restic 拿锁时不跳过陈旧锁，worker 到点被强制结束留下的独占锁会挡住
  之后所有备份。代价是有人同时用管理员身份 `prune`／`repair` 时可能误报，所以维护前先停 worker（见「维护围栏」）。
- 一次最多 20 分钟（共用包的 `CHECK_WINDOW`，生产者补「中途没了」的时限和健康页正文都从它算），超了算没做完；
  只用 worker 的追加身份和凭据。修仓库（`repair`）、保留清理（`forget`／`prune`）、
  `unlock` 都要别的身份，不在这里做。
- 作业执行器（`AppendRunner`）只认本作业的那一条备份调用，抽查不经过它，走 worker 自己的
  `repository_check` 执行器：同一套固定包、凭据、空环境、进程组回收和时钟检查。

## 结果只分三类

restic 的退出码和 summary 分不出「仓库坏了」和「连不上、凭据不对」，所以只分三类：

| 结果 | 判据 |
|---|---|
| 干净 | 退出 0，stdout 恰好一条 `summary`，`num_errors` 为 0、`broken_packs` 为空 |
| 查出问题 | 那条 `summary` 里错误数或损坏数据包个数大于 0，不论退出码；只记两个个数，不记包 ID |
| 没做完 | 让给备份作业、超过 20 分钟、输出越界（stdout 8 MiB、stderr 4 MiB）、输出和固定版本对不上、装配不起来、worker 中途没了 |

stdout 每个非空行都必须是 JSON 对象；没有 summary、不止一条、字段类型不对、退出非 0 却一个错误都没报，
都按「输出和固定版本对不上」算没做完。stderr 只计数，内容不留。子进程清理没能证实时不上报，worker 退出。

## 节奏和组号：生产者只看账本

worker 不存进度，状态文件不加字段。节奏和组号全由人侧生产者从账本里的 `backup.repository_check` 事件算
（`crates/local-server/src/backup/checks.rs`），进程重启后重读账本得到同样的结论：

- **轮转**：距上一次轮转的原意图满 24 小时，查上次那组的下一组（30 之后回到 1，第一次查第 1 组）。
  结果好坏都往下轮，一组一直不干净也不会卡住整圈。
- **重查**：一组最近一次结果不干净（查出问题或没做完），第一次不干净后 1 小时重查，连续两次以上后每 24 小时
  重查一次，直到干净。几组都要重查时先查结果最早的那组；两次重查的原意图之间至少隔 1 小时。
  重查和轮转同时到期先重查。
- **不派**：在封存、有作业没收口、上一组还在查（原意图落账不到 30 分钟）、本地时钟早于账本里的抽查时刻。
- **中途没了**：原意图落账满 30 分钟（抽查窗口加 10 分钟余量）还没有结果，下一次有人问时生产者补一条
  「没做完（worker 中途没了）」。补过之后 worker 迟到的结果被拒，这一组按重查节奏再派。
- 检查序号单调递增、不复用，和备份作业序号分开；别的目标类别的检查也占序号。

## 账本事件

动作 `backup.repository_check`，事件键 `backup.repository_check:<检查序号>`，原意图和结果共用一个键：

| 阶段 | 字段 |
|---|---|
| 原意图 | `check_sequence`、`bucket`、`buckets`（固定 30）、`purpose`（`rotation`／`retry`）、`target_kind`、`phase_note` |
| 结果 | `check_sequence`、`bucket`、`buckets`、`result`、`reason`、`errors`、`broken_packs`、`repository`、`recorded_by` |

结果由 worker 写时 `recorded_by` 是 `worker`、`repository` 是仓库地址的 SHA-256（带固定前缀，不含地址本身）；
由生产者补「中途没了」时是 `producer`、`repository` 为 `null`。worker 不能报「中途没了」，生产者只能报它。
读回时按固定写法重新生成整条事件逐字段比对，对不上的忽略。

**换仓库**：worker 报的仓库摘要变了，旧仓库各组的结果不再算，首轮覆盖期从这一刻重新起算。
同一个地址上重新 `init` 的仓库摘要不变，旧结果会沿用：这是已知局限，重建仓库后要人工核对健康页。

## 消息

同一个备份 socket 上多了一个队列命令和一种上报消息，和原有消息字段都不同、互相解不出：

- 队列命令 `repository_check`：回 `check_assigned`（原意图回执）、`outstanding` 或 `not_due`。回执要通过完整性
  核对、是原意图阶段、目标类别是固定目标。
- 上报 `RepositoryCheckReport`：`{schema, target, check, outcome, repository}`，只能导出唯一一条结果事件。
  生产者核目标类别，再在账本里找同一检查键、本目标的唯一原意图（找不到、不止一条、组号对不上都拒绝），
  才追加；回执 `{schema, request_sha256, record}`，worker 核请求摘要、规范编码、回执哈希和事件本身。
  同一份上报重放回原回执；同一检查已经有别的结果（例如补过「中途没了」）就拒绝。

## worker 的空闲一轮

每一轮先问「下一个待办」，有作业先做作业。没有待办时：

1. 手里有上一次没交上的结果就先交；交上了 1 秒后看下一轮，交不上留到下一轮，留满 1 小时放弃
   （生产者之后补「中途没了」，那一组再重查）。生产者出错一律不回答复、不说原因，worker 分不出「被拒」和
   「连不上」：被拒的结果（例如账本换过、原意图已不在）也照样留满 1 小时才放弃，这期间不派新检查。
2. 否则问生产者要不要查一组；派下来就跑，跑的时候每 30 秒问一次有没有待办作业，有就停下让路，
   这一次记「没做完（让给了备份作业）」。
3. 跑完把结果交给生产者。没交上的结果只放内存里，进程没了就丢。

## 维护围栏

`prune`、`repair`、`unlock` 这些管理员操作之前，先停 worker 服务和请求封存的定时器
（`systemctl stop studiolo-backup-worker.service studiolo-backup-schedule.timer`），做完再用 `systemctl start` 起这两个；
不然带 `--no-lock` 的抽查可能和它们撞上而误报，定时器也会在维护期间不断请生产者封存、攒下待办。
误报会在重查干净后消掉，但会让健康页降级一阵。步骤和停着时健康页的样子见 `deploy/backup-worker/README.md`「维护围栏」。

## 健康页

新加一节 `backup-repository-check`「备份仓库检查」，判据见 `docs/m2/health-and-alerting.md` §2.2：
有组最近一次有结论的结果是查出问题就降级（一次就算，直到同一组重查干净）；只有读到损坏的数据包才说「损坏」，
只有错误数时说「没通过，可能是仓库损坏，也可能是连不上或凭据不对」。

## 验证

- 共用类型、命令形态、输出归类、事件形状与读回、两种新消息：`crates/backup/tests/contract/{repository_check,check_message,restic}.rs`。
- 生产者节奏与组号、补「中途没了」、换仓库：`crates/local-server/src/backup/checks_tests.rs`；
  真实账本上的派组、重放、冲突、目标核对、作业优先、挂满 30 分钟补「中途没了」后迟到结果被拒且 1 小时后重查、
  换仓库后旧组作废：`crates/local-server/tests/coordinator/checkpoint_owners/producer_check.rs`。
- 健康页各结论的正文和状态（含本地时钟早于账本时照样报能证实的过期）：`crates/local-server/src/backup/check_evidence_tests.rs`、
  `crates/local-server/src/health/tests/backup_check_details.rs`。
- worker：失败类别到「没做完」原因、让路结束整组、子进程退出后读空管道时不再问让路、stderr 上限：
  `crates/backup-worker/src/repository_check.rs`、`crates/backup-worker/tests/support/fixtures.rs`。
- 真实 CLI（显式 `--ignored`，要 `STUDIOLO_RESTIC_TEST_BINARY`）：`crates/backup/tests/restic_cli.rs` 建库、备份、
  30 组全部干净、弄坏最大的数据包后只有它那一组查出问题（`broken_packs` 为 1）、别的组照样干净；
  `crates/backup-worker/tests/support/restic_cli.rs` 经 worker 的抽查入口跑 30 组都归成干净。
  108 测试箱任务 `20261117-090709-299d` 首次跑过；独立复查第 1 轮修复后 `20261117-093320-0a55` 重跑全过。
- 架构守卫钉住新文件和新路径：`crates/studiolo/tests/architecture_support/backup/{ipc,source,process}.rs`。
