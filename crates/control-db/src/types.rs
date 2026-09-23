//! 对外的持久状态与记录类型，以及持久文本与领域值之间的换算。
use super::*;

/// 权威的任务生命周期状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 人写定义，尚未入队。
    Draft,
    /// 可被认领。
    Queued,
    /// 已被 worker 预留、运行准备尚未完成；现行 `claim` 直接写 `running`，本库不写出此状态。
    Claimed,
    /// 当前有 attempt 持有该任务。
    Running,
    /// 终态副作用已预留，必须前滚。
    Committing,
    /// 已请求取消，等待 runner 完成隔离。
    CancelRequested,
    /// 产物等待人工审阅。
    NeedsReview,
    /// 再次运行前需要人工修订。
    NeedsRevision,
    /// 已确认的输入发生变化，需要重新确认；策略变化不触发此状态（见 `uses_old_policy`）。
    NeedsReconfirm,
    /// 人工审阅已批准结果。
    Approved,
    /// 成功终态。
    Succeeded,
    /// 失败终态。
    Failed,
    /// 已取消终态。
    Cancelled,
    /// 已放弃：重启后隔离、租约巡检回收，或持有者回执声明放弃。
    Abandoned,
}

impl TaskStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Queued => "queued",
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::Committing => "committing",
            Self::CancelRequested => "cancel_requested",
            Self::NeedsReview => "needs_review",
            Self::NeedsRevision => "needs_revision",
            Self::NeedsReconfirm => "needs_reconfirm",
            Self::Approved => "approved",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, DbError> {
        match value {
            "draft" => Ok(Self::Draft),
            "queued" => Ok(Self::Queued),
            "claimed" => Ok(Self::Claimed),
            "running" => Ok(Self::Running),
            "committing" => Ok(Self::Committing),
            "cancel_requested" => Ok(Self::CancelRequested),
            "needs_review" => Ok(Self::NeedsReview),
            "needs_revision" => Ok(Self::NeedsRevision),
            "needs_reconfirm" => Ok(Self::NeedsReconfirm),
            "approved" => Ok(Self::Approved),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(DbError::Corrupt(format!("unknown task status {other}"))),
        }
    }
}

/// 持久化的 attempt 阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttemptPhase {
    /// 已认领但未启动；现行 `claim` 直接写 `running`，本库不写出此阶段。
    Claimed,
    /// runner 工作可能正在进行。
    Running,
    /// 能力已撤销，等待 runner 隔离。
    CancelRequested,
    /// 终态副作用必须幂等前滚。
    Committing,
    /// 成功结束的 attempt。
    Completed,
    /// 失败结束的 attempt。
    Failed,
    /// 已取消的 attempt。
    Cancelled,
    /// 已放弃的 attempt：重启隔离、租约巡检回收、交接后让出，或回执声明放弃。
    Abandoned,
}

impl AttemptPhase {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "claimed",
            Self::Running => "running",
            Self::CancelRequested => "cancel_requested",
            Self::Committing => "committing",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, DbError> {
        match value {
            "claimed" => Ok(Self::Claimed),
            "running" => Ok(Self::Running),
            "cancel_requested" => Ok(Self::CancelRequested),
            "committing" => Ok(Self::Committing),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "abandoned" => Ok(Self::Abandoned),
            other => Err(DbError::Corrupt(format!("unknown attempt phase {other}"))),
        }
    }
}

/// 持久化的任务投影与人写定义。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    /// 稳定的任务标识。
    pub task_id: TaskId,
    /// 人写定义；运行时字段存于独立列。
    pub definition_json: Value,
    /// 运行时状态。
    pub status: TaskStatus,
    /// 活动 attempt（若有）。
    pub active_attempt: Option<AttemptId>,
    /// 最近一次结束的 attempt（若有）。
    pub last_attempt: Option<AttemptId>,
    /// 不可变输入快照：带快照入队时写入，认领后即当前或上一次 attempt 所用的快照。
    pub input_snapshot: Option<String>,
    /// 原始创建时间，用于确定性地渲染任务页。
    pub created_at_ms: i64,
}

/// 为单个已快照源保留的元数据。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEntryRecord {
    /// Vault 逻辑路径。
    pub path: String,
    /// 精确的源修订号。
    pub revision: String,
    /// 生效标签，绝不含源内容。
    pub effective_label_json: Value,
}

/// 只含元数据的不可变快照清单。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRecord {
    /// 内容寻址的快照标识。
    pub snapshot_id: String,
    /// 所绑定 Vault 的身份。
    pub vault_uuid: VaultUuid,
    /// 各源的元数据。
    pub entries: Vec<SnapshotEntryRecord>,
}

/// 终态副作用发生前持久预留的数据。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TerminalReservation {
    /// 只含事件与 attempt 身份等元数据；历史 v1/v2 记录可能仍需迁入 staging。
    pub report_json: Value,
    /// 内容面 staging 指针 {id, sha256}；新的内联候选一律拒绝。
    pub artifacts_json: Value,
}

/// 持久化的 attempt 记录。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AttemptRecord {
    /// attempt 标识。
    pub attempt_id: AttemptId,
    /// 所属任务。
    pub task_id: TaskId,
    /// 单调递增的 fence 令牌。
    pub fence: Fence,
    /// 当前阶段。
    pub phase: AttemptPhase,
    /// worker 身份。
    pub worker_id: String,
    /// 不可变输入快照。
    pub snapshot_id: String,
    /// 租约截止时间（Unix 毫秒）。
    pub lease_until_ms: i64,
    /// 防 PID 复用的 runner 引用（若已挂接）。
    pub runner_ref_json: Option<Value>,
    /// 终态前滚输入（若已预留）。
    pub terminal: Option<TerminalReservation>,
}

/// 一条待投递的任务页投影。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OutboxRecord {
    /// 单调递增的 outbox 序号。
    pub sequence: i64,
    /// 所属任务。
    pub task_id: TaskId,
    /// 投影载荷。
    pub payload_json: Value,
}

/// 已落定的终态迁移，其权威审计结果仍待投递。
#[derive(Clone, Debug, PartialEq)]
pub struct PendingTerminalAudit {
    /// 不可变的 attempt 及其原始报告预留。
    pub attempt: AttemptRecord,
    /// T2 提交的状态，不受之后任务变化影响。
    pub status: TaskStatus,
    /// 同一事务中提交的 Broker 回执。
    pub receipt_json: Value,
}

pub(super) fn parse_task_id(value: &str) -> Result<TaskId, DbError> {
    TaskId::from_str(value).map_err(|_| DbError::Corrupt(format!("invalid task id {value}")))
}

pub(super) fn parse_attempt_id(value: &str) -> Result<AttemptId, DbError> {
    AttemptId::from_str(value).map_err(|_| DbError::Corrupt(format!("invalid attempt id {value}")))
}

pub(super) fn parse_vault_uuid(value: &str) -> Result<VaultUuid, DbError> {
    VaultUuid::from_str(value).map_err(|_| DbError::Corrupt("invalid Vault UUID".to_owned()))
}

pub(super) fn fence_i64(fence: Fence) -> Result<i64, DbError> {
    i64::try_from(fence.get())
        .map_err(|_| DbError::Corrupt("fence exceeds SQLite range".to_owned()))
}
