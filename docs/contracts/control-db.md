# Control DB schema 契约

> crate：`studiolo-control-db`。它是运行平面的 SQLite WAL 与 TaskEngine 的单写者持久边界，
> 不保存笔记正文，也不实现第二套业务状态机。所有写方法要求 `&mut ControlDb`。

## 1. 打开、配置与 Vault 绑定

```rust
ControlDb::open(path, vault_uuid) -> Result<ControlDb, DbError>
ControlDb::open_in_memory(vault_uuid) -> Result<ControlDb, DbError>
```

打开时依次配置/迁移/绑定：`journal_mode=WAL`（内存库使用 SQLite 可支持模式）、
`synchronous=FULL`、`foreign_keys=ON`、`busy_timeout=5000`、`trusted_schema=OFF`。文件默认由
组合根放在 `WORKBENCH_DATA/control.db`，必须位于 Vault 外。

`vault_binding` 首次写入 `.workbench/vault.json` 对应的 Vault UUID；后续以不同 UUID 打开返回
`VaultBindingMismatch`。`pragma(name)` 只允许 journal_mode/synchronous/foreign_keys 三项，
不允许调用方拼任意 PRAGMA。

## 2. v2 schema

| 表 | M0 持久语义 |
|---|---|
| `schema_migrations` | 已应用整数 version 与时间字段；当前 `SCHEMA_VERSION=2`，M0 applied_at_ms 记 0 |
| `vault_binding` | 单行 Vault UUID |
| `fence_counter` | 全库单调、非零 next fence；在 claim transaction 内分配 |
| `tasks` | task ID、原样人写 definition JSON、status、active/last attempt、input snapshot、时间 |
| `snapshots` / `snapshot_entries` | Vault UUID 与 path/revision/effective-label 元数据；不存 source bytes |
| `attempts` | task、唯一 fence、phase、worker、snapshot、lease、runner ref、terminal reservation |
| `processed_events` | 为后续幂等 event 留的 durable key/result；M0 尚无公开写 API |
| `projection_outbox` | 唯一 event key、task、projection JSON、delivery 时间 |
| `artifact_commits` | attempt 唯一的 Broker receipt JSON |
| `terminal_audit_outbox` | T2 同事务写入的唯一 attempt/status 与审计交付时间 |

部分唯一索引保证每个 task 最多一个 `claimed/running/cancel_requested/committing` attempt；
fence 本身全表唯一。外键由 SQLite 强制，snapshot entries 可随 snapshot 级联，attempt/
outbox/commit 证据不能因删除 task 被静默丢失。

M0 没有 task_commands、effective_policies、approvals 或 gateway_calls 公共 schema；它们随真实
M3 消费者和迁移另行增加。当前 migration 也没有 checksum/降级协议，不能把 `CREATE IF NOT
EXISTS` 冒充已实现的不可变迁移校验。v1→v2 对已终态且有 reservation/receipt 的旧 attempt
一次性补审计 outbox；重复打开 v2 不重建已交付记录，补记仍由 AuditLedger event/phase
幂等保护。历史 v1 若缺 `tasks.input_snapshot`，先检测实际列并 `ALTER TABLE`，再从
active/last attempt 的 snapshot 回填；不能只用 `CREATE TABLE IF NOT EXISTS` 留下缺列库。

## 3. 状态与公开记录

`TaskStatus`：`Draft | Queued | Claimed | Running | Committing | CancelRequested | NeedsReview |
NeedsRevision | NeedsReconfirm | Approved | Succeeded | Failed | Cancelled | Abandoned`。

`AttemptPhase`：`Claimed | Running | CancelRequested | Committing | Completed | Failed |
Cancelled | Abandoned`。当前 `claim` 在一个 transaction 中直接产生 Running；Claimed 是可读
持久枚举保留值，不构成 M0 的额外已实现阶段。

`TaskRecord` 保留 `created_at_ms` 供页面身份元数据生成，并把 `definition_json` 与
`status/active_attempt/last_attempt/input_snapshot` 分栏；代码写投影时必须遵守
[core-types.md](core-types.md) 的人写/投影字段归属。`SnapshotRecord` 只保留 metadata；
`AttemptRecord` 可带 PID-safe runner ref 和不可变 `TerminalReservation`；`OutboxRecord` 按
sequence 返回。

未知 enum、非法 ID、零/越界 fence 或损坏 JSON 返回 `Corrupt`，不能降级成宽权限状态。

## 4. M0 操作与事务线性化点

- `create_task`：插入人写 definition，初态 Draft，同时产生页面创建所需的 projection outbox。
- `queue_with_snapshot`：TaskEngine Run 的路径，在一个 transaction 保存 snapshot metadata、
  绑定 input snapshot、从 Draft → Queued 并写投影。`queue_task` 是不附 snapshot 的基础
  helper，仅供底层契约构造；TaskEngine 不用它替代确认输入。
- `require_reconfirmation`：确认后的输入变更使 Queued → NeedsReconfirm，并同步写投影；
  不创建 attempt，不自动改成新 snapshot。
- `next_queued`：按更新时间/task ID 读取最老候选，不占有它。
- `claim`：`BEGIN IMMEDIATE` 验证 Queued、Vault-bound snapshot、worker/lease；保存 snapshot
  metadata、分配 fence、创建唯一 attempt，并原子设置 task Running/active attempt/input
  snapshot，并写 projection outbox。已绑定 snapshot 时，新请求必须完全一致。后者是本次
  attempt 的冻结输入投影，不能从人写 definition 反推覆盖。
- `attach_runner` / `heartbeat`：只有 task/attempt/fence 匹配且 attempt Running 才更新；否则
  `FenceMismatch`。
- `cancel_task`：Draft/Queued 直接 Cancelled；Running 原子把 task/attempt 置 CancelRequested
  并使 task 不再通过 active guard，同时写投影。Runner 隔离由组合根完成，随后
  `acknowledge_cancel` 在匹配 fence 下收敛 Cancelled、清 active、写 last attempt 和投影。
- `is_attempt_active`：只有 task 与 attempt 都是 Running、active attempt/fence 精确匹配才真；
  Gateway guard Adapter 可据此实现 fresh 查询。

读取方法 `task`、`snapshot`、`pending_outbox` 与 `committing_attempts` 只返回 plain DTO，不
泄漏 `rusqlite::Connection`/transaction。

## 5. Committing 两阶段终态

T1 `reserve_terminal(task, attempt, fence, reservation, now)` 只接受 Running：在同一 transaction
把 attempt/task 设为 Committing，写投影，并保存不可变 runner report + artifact plan。同 fence、同
reservation 重放成功；不同内容或其他 phase 返回结构化拒绝。

T1 提交后，TaskEngine 才写 Audit intent 并调用 VaultBroker。manifest 是跨文件/DB 平面的
commit point；SQLite 不宣称与文件写原子。

T2 `finalize_terminal(..., status, artifact_receipt, ...)` 只接受 Committing，且 status 只能是
NeedsReview/Succeeded/Failed/Cancelled。它原子地：

1. 可选保存 attempt 唯一 artifact receipt；
2. 将 attempt 变为 Completed/Failed/Cancelled；
3. 清 active attempt、写 last attempt 与 task 终态，同时保留本次 Run 冻结并由 claim 绑定的 input snapshot；
4. 以 `terminal:<attempt-id>` 插入唯一 projection outbox；payload 固定携带四个系统字段
   `status/active_attempt/last_attempt/input_snapshot`，不得夹带或覆盖人写字段。
5. 有 artifact receipt 时同时插入唯一 terminal audit outbox；Outcome 只能在 T2 后声称该
   状态，成功追加后再 `mark_terminal_audit_delivered`。

相同合法状态及 receipt 重放幂等，不产生第二条 outbox；相同 attempt 改终态或 receipt
不能冒充重放。`pending_terminal_audits` 关联 reservation、status 与 receipt，
`artifact_receipt` 提供终态重放的只读结果。`mark_outbox_delivered` 仅在 Broker 确认投影后
标记；未知/已交付 sequence 返回 NotFound。审计 acknowledge 重放保持原交付时间。

## 6. 重启只前滚

`abandon_inflight` 只把 Claimed/Running 收敛为 Abandoned，清理对应 task active attempt 并写
投影；它不碰 CancelRequested 或 Committing。`cancelling_attempts` 供 TaskEngine 在确认
M0 Stub 已隔离后结束取消；`committing_attempts` 返回全部 reservation，供
TaskEngine 根据已存在 manifest 或保存的 plan 继续 T2/提交；该路径绝不能调用 Runner
`start`。具体矩阵见 [task-engine.md](task-engine.md) 与 [vertical-chain.md](vertical-chain.md)。

## 7. M0 错误与验收

`DbError` 精确为：`Sqlite`、`Json`、`VaultBindingMismatch { expected, actual }`、
`NotFound(kind)`、`InvalidTransition { from, operation }`、`AlreadyTerminal { phase, operation }`、
`UnsupportedSchema { found, supported }`、`FenceMismatch`、`Corrupt(reason)`。
唯一/锁冲突由 `Sqlite` 保留底层来源；调用方不能把所有 constraint error 吞成幂等成功。

`AlreadyTerminal` 携带结构化 `AttemptPhase`：`acknowledge_cancel` 遇到 committing /
completed / failed / abandoned 时返回它，调用方（TaskEngine）只按变体映射，不得解释
`InvalidTransition.from` 里的状态名字符串（00-overview §6）。`UnsupportedSchema` 在持久
PRAGMA和DDL前判定：迁移账本版本高于`SCHEMA_VERSION`即拒绝，迁移事务内还须独立复核。
T-029将WAL设置移至迁移成功之后；静止、干净关闭的DELETE/WAL新版库须保持主数据库
字节、schema/数据和journal配置，不自动降级或重建。此证据不承诺SQLite侧文件完全无写，
也不覆盖热journal恢复或外部并发修改。账本名称按SQLite大小写规则识别，view不能冒充表。
（2026-09-05 T-027新增错误；T-029补副作用边界。）

`open_in_memory` 是快速替身，但重启验收必须复用同一临时文件。契约测试至少覆盖：空库 v2
迁移与 pragma、错 Vault 拒绝、claim 唯一/fence 单调、claim 写入独立 input snapshot、snapshot
不含正文、T1/T2 重放只产生一个且恰含四个系统投影字段的 outbox、取消撤权、重启 abandon
Running 且保留 Committing/CancelRequested、v1→v2 审计 outbox 修复、outbox acknowledge、终态
赢家之后的取消确认返回结构化`AlreadyTerminal`、新版schema拒绝的主库/配置保持、
预检后版本推进时事务内再次拒绝、空账本/大小写/内存初始化，以及
非 TaskEngine crate 无 Control DB 写连接的架构扫描。
