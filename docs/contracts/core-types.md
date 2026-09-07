# 跨 Module 核心类型契约

> 本页记录跨 Seam 的领域类型及其 M0 代码归属。`studiolo-core` 只放无 I/O 的基础值类型；
> 为服从本轮 crate 写域，Gateway 专用 DTO 与 guard 暂留 `studiolo-gateway`，不因此复制语义。
> core 不放文件、SQLite、网络、进程或任务状态机实现。

## 1. 标识与序列化

| 类型 | M0 代码归属 | 约束 |
|---|---|---|
| `VaultUuid` | core | UUID v4；canonical Vault 初始化一次后不变 |
| `TaskId` | core | M0 生成 ULID；未来 Inbox Dispatch 的确定性映射另在 M3 实现 |
| `AttemptId` | core | 新 claim 生成的新 ULID；文本固定为 `A-<ULID>` |
| `Fence` | core | 非零、在同一 Control DB 全局单调递增的整数；只由 claim transaction 分配 |
| `Revision` | core | 32 字节 SHA-256；调用方不得解析其内部构造 |
| `ToolId` / `PermissionSet` | core | 逻辑工具名与显式允许集合；公开组合操作只有求交，不提供隐式放宽 |
| `SnapshotId` | vault-broker | 对排序后的 snapshot manifest 计算 SHA-256；调用方视为不透明值 |
| `AuditEventId` | audit-ledger | 非空且无控制字符；intent/outcome 共用的审计幂等身份 |
| `CallId` | gateway | 非空且无控制字符；Gateway 调用幂等身份 |
| `IdempotencyKey` | 各拥有写入口的 Module | 与规范化请求指纹绑定；同 key 异请求必须冲突 |

所有持久枚举使用稳定的 `snake_case` 字符串；未知值必须产生显式的
`unsupported_version`/`invalid_value`，不得静默映射成较宽权限。

## 2. 路径与 revision

`VaultPath` 是已完成语法校验的 UTF-8 相对路径：

- 非空、非绝对路径，不含 `.`、`..`、NUL、控制字符或反斜杠；
- 拒绝大小写不敏感形式的 `%2f`、`%5c`、`%00` 与编码 dot segment；
- 只接受 Unicode NFC 拼写：任何组合记号（拉丁、假名浊音符等）与分解形式（含 Hangul jamo）
  一律 `ForbiddenEncoding`，因为归一化不敏感的文件系统会把分解/组合拼写解析成同一物理文件；
  `collision_key` 在 NFC 之上再折叠大小写，Broker 对磁盘条目名先 NFC 再比较（2026-09-06 T-032）；
- 不代表文件系统对象已经安全；symlink、magic link、hardlink、文件类型、根身份与
  竞态检查仍由 VaultBroker 在目录句柄上完成；
- `.git/**`、`.workbench/**`、`.obsidian/**` 与 `.stignore` 是受保护路径，普通
  mutation 不得写入。

revision 的域分离构造为：

```text
SHA256("studiolo-revision-v1\0" || vault_uuid || "\0" || vault_path || "\0" || raw_bytes)
```

因此同字节在不同 Vault 或不同路径下不是同一 revision。revision 比较使用常量时间与否
不构成 M0 安全边界，但不得只用 mtime、大小或平台 inode。

## 3. 快照与数据标签

M0 的 `SnapshotFile` 包含 `path`、`revision` 和捕获字节；`Snapshot` 包含 Vault UUID、
排序后的 files 与内容寻址 `SnapshotId`。ID 使用版本化 SHA-256 域，绑定 Vault UUID 和
排序后的 path/revision manifest；revision 又绑定源字节。调用方为每条输入提供 expected
revision，捕获后再复核全部源 revision，任一输入不符则整个 snapshot 失败。加载时再核验
内容、排序、Vault 身份与 ID。Control DB 只存输入 metadata，不保存源正文。

`DataLabels` 是独立基础类型，M0 Snapshot DTO 尚未携带它。TaskEngine 给输入 metadata 使用
Restricted 默认，不提供基于文件自述的外发授权；完整 effective labels/policy 绑定随真实
受控 Runner 进入后续里程碑。

`DataLabels` 是不可拆分的值对象：

- `classification`: `public | internal | confidential | restricted`
- `backup_scope`: `full | summary_only | none`
- `desktop_sync`: `deny | summary_only`
- `model_egress`: `deny | local_only | allow_providers([...])`

默认未分类内容为 `restricted + desktop_sync: deny + model_egress: deny`。
`backup_scope: none` 必须同时是 `desktop_sync: deny`；空 provider allowlist 非法。

## 4. 任务状态与两阶段提交

```text
Task:    draft -> queued -> running -> committing -> needs_review
           |        |         |
           |        |         +-> cancel_requested -> cancelled
           |        |         +--reconcile----------> abandoned
           +--------+-> cancelled
                    +-> needs_reconfirm

Attempt: running -> committing -> completed
            |
            +-> cancel_requested -> cancelled
            +--reconcile----------> abandoned
```

`TaskStatus` 同时保留 `Claimed/CancelRequested/NeedsRevision/NeedsReconfirm/Approved` 等
PRD 状态。M0 TaskEngine 执行 Draft、Queued、Running、CancelRequested、Committing、
NeedsReview、NeedsReconfirm、Cancelled 与 Abandoned；claim transaction 直接把 task/attempt
持久为 Running。枚举值存在不代表审批、修订或自动重试的业务入口已经实现。

`committing` 是 M0 新增的持久中间态：Runner 已结束且提交计划已封存，但 canonical
manifest 尚未被 TaskEngine 确认为 commit point。它不是对用户宣称成功的终态。

首个成功写入 Control DB 的终态 transaction 获胜。旧 fence、取消后的结果或任何迟到
报告均不得修改终态。M0 的 `reconcile()` 不根据 retry policy 自动重跑 Runner。

## 5. task 文件字段归属

本表只适用于 agent task 页面；人手写的项目 `TASKS.md` 协调行继续以文件为权威。

| 类别 | 字段 | 规则 |
|---|---|---|
| 身份元数据 | `type`、`id`、`created` | 创建时由系统写入，之后不可变；外改不授予权限 |
| 人写任务定义 | 正文目标、`role`、`adapter`、`parent`、`project`、`input_paths`、`allow_web`、`allow_db`、`retry_policy`、`timeout` | 下次显式 Run/Revise 才进入新 snapshot；只能收窄 effective policy |
| 待确认意图 | `requested_action` | 人可写，但文件变化只生成待确认意图；不能启动、取消或批准运行 |
| TaskEngine 投影 | `status`、`active_attempt`、`last_attempt`、`input_snapshot` | 权威在 Control DB；只经 projection outbox + VaultBroker 语义 mutation 更新 |

投影 mutation 只改上述受保护顶层标量，必须保留未知 frontmatter、顺序、注释与正文。
CAS 冲突不回滚已提交的运行事实；保留人写内容、重试投影字段并告警。

## 6. M0 capability 与活跃 attempt 查询

M0 的 `JobCapability` 是 `studiolo-gateway` 内的进程内 DTO，精确绑定 task、attempt、fence、
允许工具集合与过期 Unix 秒数。部署、policy、角色三层 `PermissionSet` 再与 capability 求交；
任何层缺失工具都拒绝。它不能由任务文本、角色提示词或 Runner 自报参数扩大。

同 crate 的只读 `AttemptGuard` 每次调用都查询当前事实：

```rust
pub trait AttemptGuard: Send + Sync {
    fn inspect(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<AttemptSnapshot, AttemptGuardError>;
}
```

`AttemptSnapshot` 只返回当前 fence 与 cancelled 标志；guard 不暴露 Control DB 连接，也不能
改变任务状态。把这些 DTO/trait 后续迁入 core 属兼容性重构，不改变本轮 Gateway 语义。

M0 没有外部 HTTP/API，也未实现签名 capability、audience、nonce、snapshot/policy hash 或
用户 `AuthContext`。因此它只能用于受信任的进程内组合与测试，不能被宣称已形成远程安全
边界；这些字段随 M2/M3 的认证与传输入口补齐。

## 7. 产物与 manifest

Runner 只声明 attempt 临时目录中的候选产物；TaskEngine/VaultBroker 负责计算并验证最终
`VaultPath`、revision 与 SHA-256。固定 commit point：

```text
sources/attempts/<task-id>/<attempt-id>/manifest.json
```

manifest 必须在逻辑上关联 Vault 身份、task/attempt/fence、请求指纹、审计 intent 与按路径
排序的 artifact path/revision/SHA-256，并作为最后一个 canonical 写。M0 不冻结 private
Rust struct 的 JSON 字段布局；写入后不可变，同 attempt 同请求幂等，异内容拒绝。

## 8. 跨 Module 错误分类

具体 Module 拥有自己的 error enum，但必须可归入以下稳定类别：

| 类别 | 是否可自动重试 | 说明 |
|---|---|---|
| `invalid_input` | 否 | 语法、schema、非法状态或参数 |
| `not_found` | 否 | 路径、任务、attempt、对象不存在 |
| `conflict` | 人工/重新读后 | revision、目标存在、字段或状态冲突 |
| `denied` | 否 | 身份、capability、路径范围或策略拒绝 |
| `stale_fence` | 否 | 旧 attempt 的任何调用或回写 |
| `already_terminal` | 否 | 首个终态已提交 |
| `idempotency_mismatch` | 否 | 同 key/ID 绑定了不同规范化请求 |
| `unavailable` | 稍后 | Adapter、依赖或审计不可用；是否失败关闭由调用契约决定 |
| `indeterminate` | 仅人工决定 | 外部调用可能已发生，禁止自动重放 |
| `storage` / `integrity` | 受控恢复 | I/O、数据库损坏、manifest 或审计链失配 |
