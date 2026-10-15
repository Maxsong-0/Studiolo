# M5 黑板与 M6 租约权威

机器入口使用 `BoardTaskSpec`，M0 `TaskSpec` 只保留给受信 Stub 装配。`files_owned` 与
`acceptance_criteria` 在 JSON 中缺失即拒，空路径数组代表只读项；策略哈希不在客户端 schema，
由服务端当前 effective policy 参数冻结。对象上限 64 KiB，标准与交接文字各最多 64 项、
每项 2048 字节，路径/产物各最多 256 项。所有端点仍须先经过实例、项目、实时 policy 与审计闸门。

`create_board_task_by_instance` 在同一事务保存 request id、完整请求判别、creator、深度与约束。
相同请求返回原 task ID，改变输入返回 `IdempotencyMismatch`；父项目必须相同，子路径必须
落在父路径内，兄弟不能相交，最多 8 个直接子项，根深度 0、最大深度 3。

当前采用保守的父子认领规则：只能在父项 Draft/Queued 时创建子项；父有非终态子项时不能
claim。所有 claim 在同一 `BEGIN IMMEDIATE` 中和全部活跃路径求交，包括父子，不存在祖先
豁免。路径按 `VaultPath::collision_key` 比较，并要求目录分隔符边界，`src` 覆盖 `src/a`，
不覆盖 `src-other`。父项不能在子项未终态时结束。这不是并行委派让出/收回协议，父项应在
子项完成后认领自己的收尾工作。

`claim_board_task` 选择显式 task，首次认领前按既有 Broker 快照冻结输入。同实例重取当前有效
claim 返回相同 attempt/fence；租约已过期时不能延长、提交或获得 Gateway 授权，即使周期
对账尚未运行。`BoardView` 分开返回 task 人写定义、board 约束、creator、持有人、当前或
末次 attempt、进度和 `uses_old_policy`。策略升级不重写旧项的冻结版本。

Handoff 必须有目标、版本化规则指针、已定/未定事项、路径+revision 必读文件、完整验收标准、
禁区。规则哈希必须等于该项冻结的 effective policy hash；必读文件 revision 必须与 Broker
读取一致。handoff 写入与释放当前 claim 同事务，原 attempt 标 handed_off、工作项回 Queued；下一实例
claim 获取新 fence 并审计 handoff.accepted，信息出现在返回的 BoardView。

回执必填 completed/failed/stopped/abandoned、产物指针、冲突、未落地 patch 指针、逐项自评、
剩余事项、耗时/工具数、学到什么与 closeout。验收标准按原顺序逐条对应；完成回执只接受
全部 pass、无剩余工作、无未落地 patch。任何状态有冲突都拒绝结束，closeout 不可省略。
所有产物与验收 evidence 经 Broker 读取核对 revision+SHA-256，并扫描行首冲突标记；
未落地 patch 同样核对 SHA-256。同项目同 artifact_target，首个通过验证的 completed 回执
在 terminal transaction 取得唯一赢家，其后互斥完成请求被拒。旧 fence 不能提交回执。

编号通过 `NumberingScanner` 扫 canonical 文件名：scope 内 prefix+数字 后跟 `-` 或 `.`，
再在 DB 同一事务取 `max(真实文件最大号, 已保留最大号)+1` 并占用。客户端不能提交 max。
request id 重放保持原号，历史空号不回填。扫描失败不会退回只看登记表发号。

正文始终在 Broker 内容平面或 staging。新 terminal reservation 只保存事件/attempt metadata
和 `StagedArtifactCommit {id,sha256}`；DB 拒绝新建 inline artifact 数组。打开 v3 DB 不会
静默删除旧 v1/v2 reservation：Engine 启动/对账先把旧计划耐久写入 Broker staging，再用
原值 CAS 替换 DB JSON。Broker 不可用则保留旧计划并失败关闭。这是逻辑内容迁出，不宣称
擦除了 SQLite 历史页/WAL/备份；原 M0 body 在迁移成功前仍存在，不能假称迁移已完成。

`reconcile_startup` 仅在组合根确认旧 Runner 已隔离后使用；`reconcile_periodic` 只撤销租约
过期、执行体 stalled、取消宽限到期的 attempt，健康任务继续运行。执行体（非 consumer）
连续 10 分钟 tool_calls/edits 不变且 edits=0 算 stalled；计数下降拒绝。CancelRequested
立即撤销工具能力，3 分钟内只收 stopped，过时由对账保存 system abandoned receipt，未验
标准逐项标 unverified。系统强制放弃回执可没有用户 closeout，不冒充正常完成。

启动失败只有首个工具前可进入 launch_failure 分支；retry_on 显式包含该值、max_attempts
允许时，最多再试两次且等待 backoff。每次重试获得新 fence。任务失败不自动重试。

权威事实与 `runtime_audit_outbox` 同事务保存，审计失败保留待交付事件；Engine 的创建、
claim、handoff、receipt、number、进度、对账和启动失败在 effect 前写 Intent，成功后交付
outbox 与 Outcome。外层仍必须校验每个调用者的实例/祖先吊销、项目、策略与请求幂等。

外层授权复核是三态（BUG-032）：放行；依据读到的数据确定拒绝（类型失效、实例或父链无效、
吊销、能力集或项目不符），私有入口 `/internal/board` 回 409 并带原因；身份库、策略库或黑板
控制存储这一刻读不出来，回 503 `board_unavailable`，什么都没写、可稍后重试；正文不带原因，
原因记进这次请求的 `board.request` 结果审计。实例链只有 `InvalidParent` 算「实例无效」。
返回前的复核在结果审计之后，它读不出来时不另记审计：读操作回 503；写操作已提交，回 409
说明已提交、结果未返回，调用方刷新黑板核对，不直接重试。

`project_board` 只更新 `<!-- studiolo:board:start -->` 与 `<!-- studiolo:board:end -->`
之间新建的受控块。重复/不配对标记拒写；原有 TASKS 人工行、frontmatter 与未知字段保留。
DB 只记末次系统块 hash；块内被人修改或首次接入未知旧块时，原内文迁至「保留的带外黑板修改」
人工区并记录耐久审计，随后生成新的系统块，不能直接覆盖人的行。
黑板项只走 Broker batch 预览/提交，成功后按已观察序号确认本项目 projection outbox；不走
M0 ops/tasks 的逐页 mutate。原 M0 项仍沿用既有四字段 CAS。受控块与机器视图都不是
回滚人工工作项的理由。

验收源：ADR-014/017/018、PRD v2 用例 38–51。新增契约位于
`crates/task-engine/tests/blackboard_contract.rs` 和
`crates/control-db/tests/blackboard_transactions.rs`；本文件描述代码合同，不代表真实 Runner
已安装、生产已启用或线上 M5/M6 验收已完成。

M6 的 `stage_board_runner_candidates(RunnerReport,now)` 只接已知 Runner attempt 的 patch 与
元数据，校验 patch SHA、base/after tree 和冲突标记后按原 T1/T2 发布 manifest。Running 且
租约有效才进入提交；取消、过期或内容拒绝保留 staging 指针，不授予工具权或终态权。
T2 状态为 NeedsReview，active_attempt 清空、last_attempt 保留；重启只前滚提交。
NeedsReview 仍是未验收子项，阻止父项结束，也不能重新 claim。AI return 拒绝对此状态收尾，
Runner 自报 completed 同样被拒。组合根仅在人侧 L3 调用 `review_board_candidates`，必须
带全量精确候选指针、真实 revision/hash、closeout 与逐标准结论，才能接受、拒绝或取消候选。
接受才成为 Succeeded；所有审阅结论都保留候选指针与发布收据。

冻结 `BoardView` 字段为 task、board、uses_old_policy、holder、progress、attempt、
candidate_stages、candidate_receipt。holder/progress/attempt 指向当前或最近一次尝试，
是否活跃须同时看 active_attempt、phase 与租约。candidate_stages 是
`[{attempt_id,fence,pointer:{id,sha256},reason}]`，reason 目前为 retained/rejected/lease_expired/
cancelled_or_inactive；candidate_receipt 是 null 或
`{manifest_path,manifest_revision,artifacts:[{path,revision,sha256}]}`。
候选恢复与人侧验收反例见 `crates/task-engine/tests/runner_candidates_contract.rs`。
