//! TaskEngine 接缝对外暴露的错误类型，以及 Control DB、审计账本错误到它的映射。
use super::*;

/// TaskEngine 接缝对外暴露的失败。
#[derive(Debug, thiserror::Error)]
pub enum TaskEngineError {
    /// 人写定义违反公开 schema，或黑板相关请求未通过校验。
    #[error("invalid task specification: {0}")]
    InvalidTaskSpec(&'static str),
    /// 任务或 attempt 不存在。
    #[error("task engine record not found: {0}")]
    NotFound(&'static str),
    /// 持久状态不允许请求的命令。
    #[error("invalid task transition from {from} via {operation}")]
    InvalidTransition {
        /// Control DB 观察到的持久状态（入参不合法时为原因说明）。
        from: String,
        /// 稳定的操作名。
        operation: &'static str,
    },
    /// attempt 身份或 fence 已不再权威。
    #[error("stale attempt fence")]
    StaleFence,
    /// 另一个终态迁移已经胜出。
    #[error("attempt is already terminal")]
    AlreadyTerminal,
    /// 同一事件或请求标识被以不同内容重放。
    #[error("idempotency identity was reused with different content")]
    IdempotencyMismatch,
    /// 权威审计接收端不可用；受保护的副作用已停止。
    #[error("audit ledger is unavailable")]
    AuditUnavailable,
    /// 审计校验或持久化失败。
    #[error("audit operation failed: {0}")]
    Audit(#[source] AuditError),
    /// Vault 读取或快照等 Broker 操作失败。
    #[error("Vault Broker operation failed: {0}")]
    Broker(#[source] BrokerError),
    /// 「manifest 最后写入」的产物提交失败；预留仍在，留待对账前滚。
    #[error("artifact commit failed: {0}")]
    CommitFailed(#[source] BrokerError),
    /// Control DB 在已归类迁移之外失败（含黑板规则拒绝等未单独映射的错误）。
    #[error("Control DB operation failed: {0}")]
    ControlDb(#[source] DbError),
    /// JSON 序列化或解析失败。
    #[error("task engine JSON operation failed: {0}")]
    Json(#[from] serde_json::Error),
    /// 另一个活动认领占有了请求的路径。
    #[error("paths held by {by_instance}: {paths:?}")]
    PathsHeld {
        #[doc = "持有冲突认领的实例。"]
        by_instance: String,
        #[doc = "与活动认领相交的请求路径。"]
        paths: Vec<String>,
    },
    /// 完整性校验失败：持久记录、快照或产物指针与预期不符（如终态预留与其 attempt 不匹配）。
    #[error("task engine integrity failure: {0}")]
    Integrity(String),
    /// 投影失败，例如任务页无法在不丢失作者内容的前提下更新。
    #[error("task projection failed: {0}")]
    Projection(String),
    /// 部分事件因各自任务页拒绝而仍待投递；同一轮中其他
    /// 任务的事件照常投递。
    #[error("task projection incomplete: {delivered} delivered, {} task(s) left pending", failures.len())]
    ProjectionIncomplete {
        /// 本轮投递的事件数。
        delivered: usize,
        /// 每个被隔离任务的首个被拒事件；该任务后续事件本轮跳过。
        failures: Vec<ProjectionFailure>,
    },
    /// 任务等待认领期间，已确认的输入发生了变化。
    #[error("task {0} requires input reconfirmation")]
    NeedsReconfirmation(TaskId),
    /// runner 引用无效：适配器与任务不符、缺少稳定标识，或带有 M0 不接受的 PID/启动时刻。
    #[error("invalid or mismatched runner reference")]
    InvalidRunnerRef,
}

/// 因所属任务页拒绝而留待投递的一条投影事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionFailure {
    /// 拒绝该投影的任务页所属的任务。
    pub task_id: TaskId,
    /// 仍待投递的 outbox 序号。
    pub sequence: i64,
    /// 稳定、人类可读的拒绝原因。
    pub reason: String,
}

pub(super) fn map_db(error: DbError) -> TaskEngineError {
    match error {
        DbError::NotFound(record) => TaskEngineError::NotFound(record),
        // 终态判断只认 Control DB 的结构化变体，不解释状态名字符串（契约 §6）。
        DbError::AlreadyTerminal { .. } => TaskEngineError::AlreadyTerminal,
        DbError::InvalidTransition { from, operation } => {
            TaskEngineError::InvalidTransition { from, operation }
        }
        DbError::FenceMismatch => TaskEngineError::StaleFence,
        DbError::IdempotencyMismatch => TaskEngineError::IdempotencyMismatch,
        DbError::PathsHeld { by_instance, paths } => {
            TaskEngineError::PathsHeld { by_instance, paths }
        }
        other => TaskEngineError::ControlDb(other),
    }
}

pub(super) fn map_audit(error: AuditError) -> TaskEngineError {
    match error {
        AuditError::Unavailable => TaskEngineError::AuditUnavailable,
        AuditError::IdempotencyConflict { .. } => TaskEngineError::IdempotencyMismatch,
        other => TaskEngineError::Audit(other),
    }
}
