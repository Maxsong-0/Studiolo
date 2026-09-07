# TaskEngine Interface 契约

> Module：`studiolo-task-engine`。它独占运行平面的写者，编排任务命令、冻结输入、
> attempt/fence、取消、manifest 提交、审计与投影。M0 提供同步对象安全 Interface 和使用
> 真实 Control DB 的 `PersistentTaskEngine<B, L>`，没有常驻调度器或远程认证入口。

## 1. Interface 与装配

`TaskEngine: Send` 的公开操作如下；每个修改操作使用 `&mut self`：

| 操作 | 输入与结果 | 责任 |
|---|---|---|
| `create_task` | `TaskSpec, now_ms → TaskId` | 校验人写定义并创建 Draft |
| `command` | `task_id, TaskCommand, now_ms → TaskView` | M0 的 Run/Cancel 闭合集 |
| `claim` | `ClaimRequest → Option<ClaimedTask>` | 原子认领最老 queued task，返回冻结的 RunSpec |
| `attach_runner` | `task_id, attempt_id, fence, RunnerRef, now_ms` | 保存带 fence 的 Runner 身份 |
| `heartbeat` | `task_id, attempt_id, fence, lease_until_ms, now_ms` | 只延长仍活跃 attempt 的租约 |
| `acknowledge_cancel` | `task_id, attempt_id, fence, now_ms → TaskView` | 已隔离 Runner 后确认取消终态 |
| `report` | `event_id, RunnerReport, now_ms → ReportResult` | T1、审计、Broker commit、T2 与幂等重放 |
| `reconcile` | `now_ms → ReconcileReport` | 收敛旧 attempt，前滚 reservation 与审计 outbox |
| `project_pending` | `→ usize` | 经 Broker 交付待投影事件，返回已交付数量 |
| `task` | `task_id → Option<TaskView>` | 只读查询，人写定义与系统投影分栏 |

`PersistentTaskEngine::new(ControlDb, broker, audit_ledger)` 取得依赖的独占所有权。
`control_db()`、`broker()`、`audit_ledger()` 只暴露共享只读引用；`into_parts()` 用于显式的
组合根交接/重启测试，不提供任意写 SQL 或 canonical 文件句柄。

Runner 的 `start/observe/cancel` 由组合根通过 `RunnerAdapter` 调用；TaskEngine 只产生
RunSpec、接收 RunnerRef/结构化报告。`reconcile` 没有 Runner 参数，也不能调用 `start`。

## 2. 人写定义、页面与冻结输入

`TaskSpec` 只包含 `role`、`adapter`、`input_paths`、`allow_web`、`allow_db`、`retry_policy`、
`timeout`、`project`、`parent`、可选 `title` 和 `instruction`。serde 拒绝未知字段；role、
project 非空，adapter 只能为 `stub`，timeout/max_attempts 非零，input_paths 不重复。
其他 Adapter 名返回 InvalidTaskSpec；真实 Runner 必须连同隔离能力在对应里程碑启用。

`RetryPolicy { max_attempts, backoff_seconds, retry_on }` 只记录人写意图，M0 不自动重试。
工具白名单字段不能单独授予 Gateway 权限。

task 页面固定为 `ops/tasks/<task-id>.md`，Draft 的 projection outbox 经 Broker mutation 创建；
Run 先交付待投影事件，再捕获页面。TaskSpec 的 `timeout` 在页面写作
`attempt_timeout_seconds`。TaskEngine 只把
`status/active_attempt/last_attempt/input_snapshot` 作为投影更新；身份、人写定义、未知
frontmatter、注释、顺序和正文保留。页面变更不是命令，`requested_action` 不自动执行。

Run 时冻结 task 页面和显式 input_paths，保存 Vault-bound snapshot metadata 后排队。
claim 读取该不可变 snapshot，不能以 Run 后的最新文件悄悄替换输入。快照 I/O 位于 SQLite
写事务外；snapshot bytes 存在 Broker data root，Control DB 的 snapshot 表只保存 metadata。

claim 前若源 revision 或 task 人写内容已改变，转 NeedsReconfirm 并返回
`NeedsReconfirmation(task_id)`，不分配 attempt。比较 task 页面时归一四个系统投影值，正常
投影更新不算人写输入变化；M0 不自动重新确认或排队。

M0 入口仅供受信任进程内组合调用。带用户认证、request hash、policy 版本、显式 command
幂等键与 Inbox UUID 的完整 Dispatch 属于 M3，当前 trait 不宣称提供远程授权边界。

## 3. claim 与 fence

`ClaimRequest` 显式传 worker ID、当前 Unix 毫秒和 lease deadline。空 worker 或已经到期的
lease 拒绝；没有 queued task 返回 `Ok(None)`。

ControlDb 的 claim transaction 是唯一线性化点：校验 queued、保存/核对冻结 snapshot、
分配全 DB 单调非零 fence、创建唯一 attempt，原子更新 task Running、active attempt、
input snapshot 与 projection outbox。M0 直接进入 Running；Claimed 是保留的持久枚举。

`ClaimedTask` 返回 snapshot ID 与不可变 `RunSpec`。Runner 只收到逻辑路径、revision 与
捕获字节，不能据此获得 canonical 根目录。后续 attach/heartbeat/report 必须同时匹配
task、attempt、fence；旧 fence 不得恢复活跃状态。`attach_runner` 再校验 RunnerRef，不能
经反序列化绕过身份要求。M0 只接受匹配 `stub`、非空 opaque_id、pid/start_time_ticks
都为空的引用；其他值为 InvalidRunnerRef。基础 RunnerRef 的 process 构造能力不代表
TaskEngine 已允许真实进程。

## 4. 取消与终态竞争

- Draft/Queued 的 Cancel 直接结束为 Cancelled。
- Running 的 Cancel 进入 CancelRequested，立即使 active guard 拒绝后续工具与成功报告。
- 组合根完成 `RunnerAdapter::cancel` 后调用 `acknowledge_cancel`，收敛为 Cancelled。
- 重复合法取消/确认不产生第二次终态；错误 attempt/fence 拒绝。
- Committing 已持久封存时，不以取消撤销已发生的 artifact 提交；保持既有 reservation 前滚。

M0 Stub 没有独立存活进程；restart 的旧 CancelRequested 可收敛为 Cancelled。真实 CLI 的
隔离确认、优雅/强制终止时限必须在对应 Runner Adapter 上线前补验。

## 5. report 的两阶段提交

`report` 接受稳定 event ID 和绑定 task/attempt/fence 的候选 artifact bytes。只有 Running
可以创建 reservation；相同 reservation 在 Committing/Completed 重放幂等，不同 event 或
内容返回 `IdempotencyMismatch`。旧 fence 先返回 `StaleFence`，取消或其他终态返回
`AlreadyTerminal`，迟到成功不能把任务改回成功。

正常顺序：

1. T1 在同一 DB transaction 将 task/attempt 置 Committing，保存不可变 report 与 artifact
   plan。保存的是 Runner 候选字节，不是 Vault 原文快照。
2. 以 action `task_engine.report` 追加 AuditLedger Intent；不可用则停止 canonical 提交。
3. Broker 按 task/attempt/fence/审计关联幂等写 artifact，最后写唯一 manifest，返回可验证
   `ArtifactCommitReceipt`。
4. T2 在同一 DB transaction 保存 receipt、置 NeedsReview/Completed、清 active attempt、
   保留 last attempt/input snapshot，并追加 projection outbox 与 terminal audit outbox。
5. 从 durable audit outbox 追加同一 event ID 的 Outcome，成功后确认交付。

`ReportResult` 包含 status、receipt 与 replayed。Outcome 失败时 DB 已完成 T2，调用方收到
审计错误；restart 只补 outcome，不重新调用 Runner 或 Broker。manifest 之前的 orphan
不能视为成功。Broker/审计故障不会清除已保存的 reservation 或静默改写计划。

## 6. 投影与重启

`project_pending` 按 outbox 顺序读取投影，重新读取 task 当前页面，通过顶层标量行级补丁
和 expected revision 调用 Broker；成功后才 acknowledge。写后重新解析，拒绝不能安全定位
的重复键、多行目标值或非法 YAML。CAS 冲突保留待办，不回滚 Control DB 运行事实。

每条事件最多尝试三次。即使 Broker 返回历史幂等 receipt，也须重新读取 canonical 页面，
证明当前 revision 仍等于本次目标输出后才 acknowledge；若输出已被外改，保留 outbox 并
重新基于最新页面补丁，不能把旧 receipt 当成当前已安装结果。

**逐任务隔离（2026-09-06 T-032）**：一条事件因它自己的任务页被拒（缺 frontmatter、身份不
匹配、非法 YAML、三次补丁都被外改、任务记录缺失）时，只把该任务隔离到本轮结束——它的
后续事件保持待办并被跳过以保住任务内顺序，其他任务的事件继续交付。整轮结束后若有隔离，
返回 `ProjectionIncomplete { delivered, failures }`，`failures` 每任务一条（首条被拒事件的
序号与原因）；Control DB、Broker、审计等存储层失败仍立即中止整轮。`Run` 只交付本任务自己
的待投影事件（同样按序、同样返回 `ProjectionIncomplete`），别的任务的坏页不能挡住这条命令。

reconcile 只处理已持久事实：

- 旧 Claimed/Running 变 Abandoned；旧 CancelRequested 变 Cancelled；绝不恢复运行。
- Committing 使用保存的 plan 和固定 commit identity 前滚，Broker 验证已有 artifact/manifest。
- T2 已成功而 outcome 未交付时，仅补 durable audit outbox。
- 投影通过 `project_pending` 独立重试；不因投影失败重跑任务。

`ReconcileReport` 分别报告 abandoned_attempts、cancelled_attempts、committed_attempts 与
audit_outcomes；最后一项只计本轮修复的旧终态待审计记录，不含前滚新提交随附的 outcome。
它不表示重建 Runner，也不触发 retry policy。
同 DB 文件的 restart 测试必须覆盖 Running、CancelRequested、部分提交、manifest 后、T2 后
outcome 故障以及重复 reconcile，spy Runner 的 start 次数不能增加。

## 7. 错误、替身与验收

错误按公开 enum 返回，包括输入/状态拒绝、NotFound、StaleFence、AlreadyTerminal、
IdempotencyMismatch、AuditUnavailable，以及带来源的 Audit、Broker、CommitFailed、
ControlDb、Json、Integrity、InvalidRunnerRef、Projection 和 NeedsReconfirmation。调用方
不得把任意底层错误吞成重放成功。Control DB 错误到 TaskEngine 错误的映射只按变体进行：
`DbError::AlreadyTerminal` → `AlreadyTerminal`、`FenceMismatch` → `StaleFence`、
`InvalidTransition` 原样透传；TaskEngine 不解释 `from` 字符串里的状态名（2026-09-05 T-027
起，00-overview §6）。

快速契约使用内存 Broker/AuditLedger 与 SQLite 内存库；重启契约复用同一 SQLite 文件，
真实垂直链使用 FsVaultBroker、文件 ControlDb 和 StubRunner。故障通过同一 trait 的
failing wrapper 注入，不能为了测试暴露底层 SQLite connection 或文件句柄。

最终通过情况与确切命令只记录在 [verify-log.md](verify-log.md)。
