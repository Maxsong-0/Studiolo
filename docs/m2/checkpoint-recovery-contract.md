# 一致 checkpoint 与新目录恢复契约

T-046，2026-09-13。本文冻结后续实现边界；尚无完整 BackupWorker、远端恢复点或
干净机恢复验收。权威仍为 Vault PRD 第11节与 ADR-002/010，不把 L0 当作灾难备份。
T-110（2026-09-22）补入 M4 策略库 `policy.sqlite3` 的屏障位置、水位与恢复核对规则。

## 封存边界

立即备份沿既有 L2 `backup.snapshot` 动作，审计失败关闭。先持久确认 Intent，再
由 Coordinator 暂停实际装配的全部变更入口，排空在途操作；未装配的 M4–M6 能力
明确记为未装配，不为了备份提前开放。唯一 Broker 租约和 `unresolved=false` 是
必要条件；Prepared 虽可阅读，也不能被视为已排空。

冻结覆盖 canonical mutation、历史整理、Bridge 摄入/导出及基线提交、Auth、Audit
和已启用 Control 的数据库变更。单独取得 CanonicalWorkbench 的 Mutex 不会冻结
其他数据库实例；每个存储 owner 必须参与，采用固定顺序与有界等待，不能声称
暂停 HTTP 就已阻止本地维护写者。后台补记成功一次也不等于所有 pending 已交付。

Auth 的会话检查和列表读取也会更新时钟地板、过期状态与 last_seen，冻结不能只
拦 HTTP 写方法。准入应覆盖 Auth→Audit→Auth 等完整语义操作；Audit 保持可用至
在途操作实际结束，再最后进入存储冻结。超时取消外层 future 不等于后台 IO 已排空。

已装配的 M3 AgentStore 也包含类型 token 摘要、撤权和实例登记；只有 reads 属于
可重建索引。若纳入恢复单元或共享本次 Audit 截止点，它必须参与批准的协调窗口；
未参与的独立服务不能被静默忽略，也不由备份器擅自停止。清单逐项标记 present/absent。

已装配的 M4 策略库（`policy.sqlite3`）保存规则母本、skills 与能力集的 desired／
版本／effective，以及实例／类型／全局三级吊销。它同样参与协调窗口，固定顺序为
Broker → Control → Auth → Bridge → AgentStore → Policy → Audit：接入方调用先经
AgentStore 核身份、再复核策略库，发布与吊销又写 Audit，所以它在 AgentStore 之后、
Audit 之前冻结。水位为 schema 版本、effective 版本、effective 哈希（64 位小写十六进制，
不能用 effective 版本号或发布时间顶替）与截止点是否处于全局吊销。接入方未给
`--policy` 的 M3 兼容模式里策略库未装配，清单显式标记 absent。

每个 SQLite owner 导出自己经过验证的 checkpoint：固定 schema/身份，完成并检查
WAL checkpoint 的 busy/result，使用 SQLite 自有备份机制或完整冻结后的安全复制。
目标文件及父目录同步，重开核验后才返回封存票据。不能复制在线主文件并忽略 WAL，
也不能把按次序复制的 DB/WAL/SHM 当作一致快照。复制前后固定源绑定不变。

各 owner 可用现有 rusqlite 的 backup 特征导出，不暴露 Connection 给协调器。
SQLite 自有备份只保证单库一致，不能替代跨 owner 屏障。副本中每个已 ACK 事件须
精确对应 Audit 截止点内的原事件，而非只验证 offset 小于最后 offset。

同一 checkpoint ID 的清单至少记录：

- Broker Vault UUID、逻辑 pair_id、提交水位、无未决证明。
- Control schema/Vault UUID/outbox 状态；Auth schema/主人绑定/审计锚点。
- Audit 最后 offset/hash；Bridge link/scope/共同基线/观察序号与持久门禁。
- AgentStore schema/类型 token 摘要/实例数；Policy schema/effective 版本/effective
  哈希/全局吊销。
- 固定应用及 CLI 包摘要、部署清单、秘密引用的脱敏映射。

这些水位各有含义，不能用索引 generation 或最后一次 Sync 退出时间替代。

## Inclusion plan 与传输源

先验证 ephemeral 不在任一源根内，再依据当前有效范围与 manifest 生成逐对象
inclusion plan。canonical、Control、Audit、完整范围的 L0 及部署/备份清单不可
被任意排除；raw/summary_only/none、已到期和 crypto-erase 按来源保留语义处理。
缺来源/分类/备份范围/到期元数据或 revision/hash 不匹配时整批拒绝，不按文件名猜。

只要有来源排除项，就建立专用过滤 staging；它位于 Vault 和 WORKBENCH_DATA 之外，
不会先把不允许的 raw 放入共享 L0 再排除。使用固定流式缓冲、文件数/总字节/空间
预留/冻结期限预算；不能复用把全库读入 Vec 或 JSON 的 M0 VaultSnapshotter。
硬链接活动文件不是不可变视图。

全部对象和本地 manifest 完整同步并原子封存后才能解冻。失败只清理本次自有临时
目录，不触碰活动库；传输源有明确期限，不晚于最短来源保留期，不能形成隐藏恢复点。

## 独立 Worker 与真实恢复点

BackupWorker 只消费封存票据所指的只读传输源，独占备份仓库追加凭据；App/Broker/
Auth/Audit/TaskEngine 不持该凭据。恢复与保留清理使用独立身份。固定 restic 包、
封闭参数和受控秘密注入，不能接收任意 shell 命令。

状态必须区分排队、封存、上传、验证、最终清单提交、成功、失败、结果未知和审计
待确认。上传退出0只是中间证据；只有远端对象完整、校验通过、成对 checkpoint
引用的最终 manifest 持久提交后，才有可恢复点。未知结果沿原作业键核对。

远端已确认而 Outcome 审计失败时保留该真实状态和固定补记，不能声称上传没发生，
也不能推进界面成功指针。此 Outcome 在 checkpoint 截止后发生，不应伪称已包含
在本次 Audit 数据库快照内。

至少保存 `checkpoint_at`、`remote_verified_at`、`finalized_at`。恢复点年龄从内容
截止时刻计算；时钟回退或时间证据缺失显示未知。默认10分钟调度加5分钟完成预算
必须满足15分钟RPO；首次完整 seed 校验前显示“尚未建立灾难恢复点”。每日追加/
不可变恢复点单独计龄，普通 SMB/NFS 目录不能显示具备不可变保护。

## 新目录恢复

Broker 主 binding 与 L0 绑定含设备/inode；复制到新目录后普通 `open_existing`
会拒绝。逻辑 pair_id 还被 mutation 日志、幂等终态、recorded envelope 和历史
目录引用，不能只替换主 binding 或生成新 pair_id。

专用恢复能力先完整验证封存内容与所有逻辑引用，再保留已验证的 Vault UUID /
pair_id，重新建立新目录的物理绑定并记录恢复 lineage。它必须由 Broker 实现，
不允许 CLI 手改 JSON、普通 initialize 自动修复，或复用单事务维护票据。
中途失败只能留下隔离且不能服务的新目录，不能发布半配对活动根。

新物理绑定须覆盖两处主 broker-binding、history/meta-v1 目录组以及已启用的
maintenance-v1，不能只重写主根设备号。日志、原幂等键与历史保持原逻辑 pair_id。

仅本机 TTY/localhost 的 restore bootstrap 使用 recovery kit 中独立、一次性、
有期限的恢复身份；任何恢复写入前建立带完整性保护且不含秘密的 transcript。
恢复到新目录，保留旧目录。先在同步和真实执行 Adapter 禁用下检查内容、版本、
权限、哈希、审计连续性和空间，再恢复主人与密钥映射。

Auth 自有恢复流程必须使旧会话/L3失效，并防止旧快照里的恢复码直接重新生效；
不能删表绕过已消费记录和审计锚点。保持原主人绑定，重新认证后才能最终切换。
旧快照恢复还需新的签发代次，防止回滚后的序号复用旧 event_id；若包含 AgentStore，
其旧类型 token 和实例同样失效，不能使快照之后撤销的凭据重新有效。
若包含策略库，恢复后必须先核对吊销再启用：快照截止点之后发生的实例／类型／全局
吊销要重新生效，未核对即 `PolicyRevocationsNotReconciled`，因为旧快照可能让已吊销
的实例或类型复活；吊销的解除只走 L3 发布，不能靠回到旧快照。
已发生的 bootstrap 步骤导入恢复后的 Ledger；尚未发生的切换不能预记成功。
最终切换先记 Intent，切换后真实记录 Outcome；失败则回滚或保持只读停服。

桌面与恢复点差异先隔离、人工协调；新 checkpoint 和写入/备份链重新验收、且纳入的
策略库已核对吊销后，才能启用双向同步。旧目录保留至回滚窗口结束。recovery kit
独立加密保存固定包、摘要、主人/秘密映射、一次性材料和 Sync 端到端恢复资料，
不进入普通 checkpoint。

## 必须留下的故障与实机证据

重点验证第二写者仍运行、WAL busy、封存前来源改变、磁盘满、上传中断、最终远端
manifest提交后ACK丢失、审计Outcome失败、新物理绑定中断，以及旧 Cookie/恢复码/
镜像基线不能在恢复后直接继续。RTO验收还需记录CPU、内存、数据量、文件数、磁盘
和NAS吞吐；没有真实干净机演练，不宣称四小时RTO通过。

NAS专用目标、追加/独立恢复身份、实际不可变能力、带外告警和独立kit介质仍待
接入核验。第二故障域已明确不配置，界面按权威说明未覆盖范围，不新增采购前提。
