# 一致 checkpoint、独立 BackupWorker 与新目录恢复（实施记录）

T-048 B1，2026-09-17。权威边界仍是 `checkpoint-recovery-contract.md`；
本页只记录**已经实现到哪一步、由哪个文件承担、留了哪些明确未验证**。
本页不宣称四小时 RTO 通过，也不宣称任何 NAS／不可变能力已具备。

## 装配总览

| 关注点 | 位置 |
| --- | --- |
| 跨 owner 屏障与封存票据 | `crates/backup/src/checkpoint/{types,barrier,manifest,inclusion}.rs` |
| 跨 owner 共享类型与 Interface | `crates/core/src/checkpoint.rs`（纯类型，零 IO） |
| 独立 Worker 状态机与固定 restic 参数 | `crates/backup/src/worker/{state,job,restic,recovery_point}.rs` |
| 恢复 kit 与 transcript | `crates/backup/src/recovery/{kit,transcript}.rs` |
| 唯一真实 IO | `crates/backup/src/store/storage.rs` |
| 新目录物理重绑定 | `crates/vault-broker/src/restore/**`（Broker 自己实现） |
| 主人审批（默认关闭） | `crates/host-config/src/backup/**` |
| 只读界面投影 | `crates/local-server/src/backup/**` |

`crates/backup` 只依赖 `studiolo-core` 与 `studiolo-audit-ledger`，
不持有任何 owner 的数据库连接，只消费封存票据所指的只读传输源。

## 跨 owner 屏障

固定顺序写在 `CheckpointOwner::FREEZE_ORDER`：
Broker → Control → Auth → Bridge → AgentStore → **Audit**。
Audit 排最后，是因为其余 owner 的在途操作（`Auth→Audit→Auth` 这类完整语义操作）
必须能走完，Audit 才进入存储冻结。

六个 owner 必须全部登记，未装配的也要登记为 `Absent`：
只登记 Broker 一个参与者会直接被 `OwnerMissing` 拒绝——
「单独拿 canonical 的 Mutex」在这里连屏障都组装不起来。
排空判定看 `DrainProof` 的四项：在途操作为 0、后台补记队列为 0、
本地维护写者已挡、Auth 的隐式读写已挡。
「暂停 HTTP」只挡住 HTTP，本地维护写者仍然是写者，所以不算排空；
「后台补记成功过一次」不等于 pending 为 0；
`prepared_readable` 只作事实记录，不放宽排空判定。
等待有界：每个 owner 的冻结耗时超过 `FreezeBudget` 即 `FreezeTimeout`，
即便该 owner 自称已排空也一样；失败时按逆序解冻已冻结的 owner，
未装配的 owner 一次都不碰。

导出票据要求 `SealedOwnerCheckpoint::is_verified()`：
WAL checkpoint 的 `busy=0` 且已 checkpoint 帧数等于总帧数、
目标文件与父目录都已 fsync、副本重开后已核验、复制前后源绑定未变。
任何一项不成立都是 `Unverified`，不会变成封存票据。

## 清单

`CheckpointManifest` 逐项要求各 owner 自己的水位：Broker 的 Vault UUID／逻辑
`pair_id`／提交水位／无未决；Control 的 schema／Vault UUID／outbox；
Auth 的 schema／主人绑定摘要／审计锚点；Audit 的最后 offset 与链哈希；
Bridge 的 link／scope／共同基线／观察序号与持久门禁；
以及固定应用与 CLI 包摘要、部署清单、脱敏秘密引用。

「不能用索引 generation 或最后一次 Sync 退出时间替代」是可执行的：
摘要字段必须是 64 位小写十六进制，**纯十进制数字**会被单独判成
`WatermarkSubstituted`（而不是笼统的格式错误），因为索引 generation
和退出时间戳恰好就长这样。Bridge 的观察序号为 0 或门禁未关是 `BridgeGateOpen`。

已 ACK 事件的覆盖要逐条对应原事件：`AuditCoverageProof` 里
`offset_comparison_only = true`（只比了 offset 小于最后 offset）直接判不成立。

秘密引用只保留「去哪拿」：环境变量名必须是大写／数字／下划线，
含 `=`、空白或小写字母的写法一律拒绝，值永远不进清单。

## inclusion plan

先证明 ephemeral 根不在任一来源根之内，再逐对象定去留。
缺来源分类、缺备份范围、缺到期元数据、缺 revision 证据或 revision 不匹配，
都是**整批拒绝**并附具体路径与原因；不按文件名猜任何一项缺失的元数据。
canonical／Control／Audit／完整范围 L0／部署清单／备份清单是必含类别，
其备份范围不是 `Full` 就是 `MandatoryExcluded`。
普通来源资料按 `BackupScope`、到期与 crypto-erase 分别落到
完整对象／脱敏摘要／非正文墓碑。

只要出现任何非「完整对象」的处理方式，`requires_filter_staging` 就为真，
此时传输源必须是 `FilteredStaging`、位于 Vault 与 `WORKBENCH_DATA` 之外、
用固定流式缓冲复制、并带齐文件数／总字节／空间预留／冻结期限四项预算。
对活动文件建硬链接被显式判为 `HardLinkIsNotImmutable`。
传输源期限晚于最短来源保留期会被判成隐藏恢复点。

## 独立 Worker

追加凭据只属于 `CredentialHolder::BackupWorker`；App／Broker／Auth／Audit／
TaskEngine 任一持有都直接拒绝组装。恢复用 `ResticIdentity::Restore`、
保留清理用 `RetentionCleanup`，与追加身份分开，身份与命令不匹配即拒绝。

restic 包名与版本写死在代码里；参数向量由封闭的 `ResticCommand`
生成，可变部分只有 tag、快照 ID 与绝对路径，每一项都过封闭字符集检查
（控制字符、空白、`;&|$\`<>(){}[]*?!\\'"` 全部拒绝）。
没有任何入口可以传入任意 shell 命令。秘密只经
`RESTIC_PASSWORD_FILE` / `RESTIC_REPOSITORY_FILE` 注入，代码里只出现变量名。

状态机区分：排队／封存／上传／验证／最终清单提交／成功／失败／
**结果未知**／**审计待确认**。上传退出 0 只把作业推进到「验证」；
必须沿原作业键在远端真的定位到对象、且对象完整＋校验通过＋
最终 manifest 成对引用本次 checkpoint，并持久提交之后，才有可恢复点。
未知结果只能用**原作业键**核对，换个键是 `JobKeyMismatch`。

远端已确认而 Outcome 审计失败时进入 `AuditPending`：
保留远端已确认这个真实状态、保留已提交的最终清单、
**不推进成功指针**，等固定补记成功再转 `Succeeded`。
Outcome 的载荷显式带 `recorded_after_checkpoint_cutoff: true`，
不伪称它包含在本次 Audit 数据库快照内。

## 恢复点年龄

持久保存 `checkpoint_at`／`remote_verified_at`／`finalized_at`。
年龄一律从内容截止时刻（`checkpoint_at`）起算，不从提交完成时刻起算。
当前时刻早于内容截止时刻即判为时钟回退，显示未知；缺时间证据同样显示未知，
都不显示 0，也不显示绿色。
默认 10 分钟调度 + 5 分钟完成预算 = 15 分钟，刚好满足 RPO；相加溢出判不满足。
首次完整 seed 校验通过前一律显示「尚未建立灾难恢复点」，
而 `mark_seed_verified` 要求确实存在成功作业与已提交清单。
每日追加／不可变恢复点单独计龄。
普通 SMB/NFS 目录与本机临时仓库**永远**显示「未证明具备不可变保护」，
主人配置里对这两种形态声明 `claims_immutability` 会被配置层直接拒绝。

## 新目录恢复

由 Broker 的 `restore` 模块实现，入口是 `rebind_new_roots`。
它先取新 canonical 根的排他 flock（第二写者仍在运行即 `NewRootBusy`），
再完整验证封存内容与所有逻辑引用：marker、两处主 `broker-binding.json`、
`writer-state.json`、全部幂等回执、`history/meta-v1` 与已启用的 `maintenance-v1`
必须都指向同一个逻辑 `pair_id`，且没有 `maintenance-v1.preparing` 残留。

验证通过后保留已验证的 Vault UUID 与逻辑 `pair_id`，按固定顺序重绑定：
lineage(`restore-v1/lineage.json`，先写 `Rebinding`) →
`history/meta-v1/binding.json`（root/requests/catalogs/outbox 四个身份全换）→
已启用的 `maintenance-v1/binding.json` → data 侧主 binding →
**最后**才是 canonical 侧主 binding → lineage 改写为 `Completed`。

这个顺序就是隔离保证：canonical 侧主 binding 被改写之前，
两处 binding 要么都是旧身份、要么互相不一致，`open_existing` 一律以
`WriterBindingMismatch` 拒绝。中途失败只留下隔离且不能服务的新目录，
不会发布半配对活动根。接手上一次中途失败的目录必须显式
`resume_interrupted = true`；此时只容忍「只差物理身份」的不一致，
逻辑身份（格式版本／`pair_id`／Vault UUID）仍必须完全一致。

明确不提供也不依赖的路径：CLI 手改 JSON、普通 `initialize` 自动修复
（`FsVaultBroker::open` 对复制过来的配对同样失败）、复用单事务维护票据。
旧目录保持原样不动。

`history/meta-v1` 与 `maintenance-v1` 的绑定记录由各自 Module 私有持有，
恢复用**字段名与 `deny_unknown_fields` 完全一致的等价定义**读写同一份字节；
`restore/tests/drift.rs` 用真实初始化出来的字节反序列化这两个定义，
上游一改字段就会立刻变红。

## recovery kit

一次性、有期限的恢复身份，只接受本机 TTY 或 localhost，
`RemoteNetwork` 直接拒绝且不消耗身份。kit 必须独立保存在普通 checkpoint 之外。
兑换成功即建立 transcript，并先记下 `KitOpened`；
没有 transcript 就拿不到恢复写入授权。transcript 是逐条哈希链，
载荷复用审计的 `SanitizedPayload` 做秘密字段检查，改任何一条都会断链。

前置检查要求同步与真实执行 Adapter 都已禁用，且内容／版本／权限／哈希／
审计连续性／空间六项全过，才允许恢复主人与密钥映射。
Auth 恢复必须使旧会话与 L3 失效、阻断旧快照里的恢复码、
保留已消费记录与审计锚点（不许删表绕过）、保持原主人绑定并重新认证、
签发代次严格前进；纳入 AgentStore 时旧类型 token 与实例同样失效。
双向同步在差异隔离、人工协调、新 checkpoint 验收、写入/备份链重新验收、
旧目录仍在回滚窗口内这五条全满足之前不得启用。

## 存储

唯一做真实 IO 的文件是 `crates/backup/src/store/storage.rs`。
它只在一个**已存在**的私有目录（0700、属当前有效用户）里操作四个固定名字：
`backup-state.json`、`backup-state.json.tmp`、`manifest.json`、`manifest.json.tmp`。
目录句柄在 `open` 时一次性取得并持有，之后所有操作都相对该句柄，
不再解析路径；不创建目录、不删除目录、不执行任何外部进程。
写入一律「固定临时名 → 写满 → fsync → 原地改名 → fsync 目录」。
半截 JSON、未知字段、未知 schema 版本、作业序号越过分配器、
符号链接冒充固定文件、目录权限被放宽，全部失败关闭。

## 明确未验证（不要当成已通过）

- **没有 NAS 专用目标、没有真实 restic 仓库、没有真实不可变能力**。
  默认 runner 是 `UnconfiguredResticRunner`，一律返回
  `Unverified { NoRealRepository }`，作业落到 `Failed { TargetUnverified }`，
  界面显示「尚未建立灾难恢复点」。完整状态机是用本地假 runner／假探针跑通的。
- **没有干净机演练，不宣称四小时 RTO 通过**，也没有记录 CPU／内存／数据量／
  文件数／磁盘与 NAS 吞吐。
- **真实 ENOSPC 未在真实满盘上触发**：磁盘满这条走的是存储层错误注入
  （`StoreError::OutOfSpace` → `FailureReason::OutOfSpace`），
  `storage.rs` 里对 `NOSPC`/`DQUOT` 的映射本身没有真机证据。
- **`maintenance-v1` 的重绑定用的是按其固定绑定形状装配的已启用状态**，
  不是走真实离线维护启用流程产生的目录。
- 第二故障域已明确不配置，本页不新增采购前提。
