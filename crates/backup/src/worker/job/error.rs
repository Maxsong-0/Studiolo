//! BackupWorker 的失败模式，以及各层错误到它的映射。

use super::super::restic::ResticError;
use super::super::state::StateError;
use crate::checkpoint::manifest::ManifestError;
use crate::store::model::StoreError;
use studiolo_audit_ledger::AuditError;

/// BackupWorker 的失败模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkerError {
    /// 追加凭据被 BackupWorker 之外的组件持有。
    CredentialHolderForbidden,
    /// 审计写入失败；Intent 阶段失败关闭。
    Audit(AuditError2),
    /// 持久存储失败。
    Store(StoreError),
    /// 状态机拒绝。
    State(StateError),
    /// 清单组装失败。
    Manifest(ManifestError),
    /// restic 参数或身份不合法。
    Restic(ResticError),
    /// 作业不存在。
    JobNotFound,
    /// 尚无成功作业与已提交清单，不能标记 seed 已校验。
    SeedNotVerifiable,
}

/// 审计失败的可复制摘要；`AuditError` 本身不是 `Copy`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditError2 {
    /// 审计存储不可用或写入被拒绝。
    AppendFailed,
    /// 同一事件键被用于语义不同的内容。
    IdempotencyConflict,
}

impl From<AuditError> for AuditError2 {
    fn from(error: AuditError) -> Self {
        match error {
            AuditError::IdempotencyConflict { .. } => Self::IdempotencyConflict,
            _ => Self::AppendFailed,
        }
    }
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CredentialHolderForbidden => {
                formatter.write_str("追加凭据只能由 BackupWorker 持有")
            }
            Self::Audit(error) => write!(formatter, "备份审计失败：{error:?}"),
            Self::Store(error) => write!(formatter, "备份状态存储失败：{error}"),
            Self::State(error) => write!(formatter, "备份状态机拒绝：{error}"),
            Self::Manifest(error) => write!(formatter, "清单组装失败：{error}"),
            Self::Restic(error) => write!(formatter, "restic 调用不合法：{error}"),
            Self::JobNotFound => formatter.write_str("作业不存在"),
            Self::SeedNotVerifiable => formatter.write_str("尚无成功作业与已提交清单"),
        }
    }
}

impl std::error::Error for WorkerError {}

impl From<StoreError> for WorkerError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<StateError> for WorkerError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl From<ManifestError> for WorkerError {
    fn from(error: ManifestError) -> Self {
        Self::Manifest(error)
    }
}

impl From<ResticError> for WorkerError {
    fn from(error: ResticError) -> Self {
        Self::Restic(error)
    }
}
