//! 同一 Broker 的类型化文件操作；主 v5 日志、固定范围绑定与双审计确认不构成第二写者。
mod atomic;
pub(crate) mod rename_capacity;
pub(crate) mod rename_commit;
pub(crate) mod rename_enable;
pub(crate) mod rename_evidence;
pub(crate) mod rename_model;
pub(crate) mod rename_prepare;
pub(crate) mod rename_preserve;
pub(crate) mod rename_review;
pub(crate) mod rename_source;
pub(crate) mod rename_target;
pub(crate) mod rename_terminal;
pub(crate) mod rename_validate;

mod audit;
mod audit_model;
pub(crate) mod binding;
mod capacity;
mod commit;
mod disk;
mod evidence;
pub(crate) mod finish;
mod frozen;
mod install;
mod limits;
pub(crate) mod maintenance;
mod maintenance_review;
mod model;
mod observation;
mod prepare;
mod prepared;
mod query;
pub(crate) mod record;
pub(crate) mod request;
mod schema;
mod status_model;
pub(crate) mod store;
pub(crate) mod terminal;
#[cfg(test)]
mod tests;
mod ticket;
pub(crate) mod tombstone;
mod usage;

use crate::{BrokerError, FsVaultBroker};
pub use audit_model::*;
pub use limits::FileOperationLimits;
pub use model::*;
pub use status_model::*;
use std::sync::Arc;
pub use tombstone::{TombstoneEntry, TombstonePage};

/// 同实例封闭票据；不能克隆或从用户输入、磁盘 DTO 构造。
pub struct FileOperationTicket {
    instance: Arc<()>,
    sequence: u64,
    fingerprint: String,
    preview: FileOperationPreview,
    intent: FileOperationAuditEvent,
}
impl FileOperationTicket {
    /// 包括已固定的 CAS 拒绝；prepare 返回不等于允许执行。
    pub fn preview(&self) -> &FileOperationPreview {
        &self.preview
    }
    /// 本次固定意图，调用方须实际提交并核验完整 AuditRecord。
    pub fn intent(&self) -> &FileOperationAuditEvent {
        &self.intent
    }
}
/// 待投递的固定事件候选；不接受调用方替换动作、主体或效果。
pub struct FileOperationAuditCandidate {
    instance: Arc<()>,
    event: FileOperationAuditEvent,
}
impl FileOperationAuditCandidate {
    /// 不含正文、原始幂等键或认证凭据的固定事件。
    pub fn event(&self) -> &FileOperationAuditEvent {
        &self.event
    }
}
pub(crate) fn digest(value: &impl serde::Serialize) -> Result<String, BrokerError> {
    serde_json::to_vec(value)
        .map(|bytes| crate::sha256(&bytes))
        .map_err(|_| BrokerError::CorruptWriterState)
}
pub(crate) fn phase(value: FileOperationPhase) -> crate::WriterPhase {
    match value {
        FileOperationPhase::Preparing | FileOperationPhase::Prepared => {
            crate::WriterPhase::Prepared
        }
        FileOperationPhase::DetachIssued
        | FileOperationPhase::InstallIssued
        | FileOperationPhase::TargetInstallIssued
        | FileOperationPhase::TargetInstalled
        | FileOperationPhase::SourceDetachIssued => crate::WriterPhase::Installing,
        _ => crate::WriterPhase::Completed,
    }
}
pub(crate) fn legacy_key(broker: &FsVaultBroker, key: &str) -> Result<(), BrokerError> {
    let state = crate::mutation::load(broker)?;
    if state
        .operation
        .as_ref()
        .is_some_and(|op| op.key_hash == key)
        || crate::recorded::store::load_id(broker, &crate::recorded::store::id(broker, key))?
            .is_some()
        || crate::mutation::replay::load(&broker.data, key, state.pair_id)?.is_some()
    {
        return Err(BrokerError::IdempotencyConflict);
    }
    Ok(())
}
pub(crate) fn guard_legacy_key(broker: &FsVaultBroker, key: &str) -> Result<(), BrokerError> {
    let state = crate::mutation::load(broker)?;
    if state
        .file_operation
        .as_ref()
        .is_some_and(|op| op.frozen.key_hash == key)
    {
        return Err(BrokerError::IdempotencyConflict);
    }
    if state.file_operations_enabled
        && request::load(broker, &frozen::operation_id(state.pair_id, key))?.is_some()
    {
        return Err(BrokerError::IdempotencyConflict);
    }
    Ok(())
}
