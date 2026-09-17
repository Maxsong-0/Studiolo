//! 严格恢复只提供两阶段固定操作；审计回调不进入 Broker，票据不能反序列化。

use super::{MutationKind, MutationMetadata, store};
use crate::content_audit::{delivery, store as events};
use crate::{
    BrokerError, ContentAuditEvent, FsVaultBroker, Mutation, MutationReceipt, Precondition,
    Revision, mutation, sha256,
};
use std::sync::Arc;

/// 固定恢复请求的同实例票据；须确认原 Intent 并最终复验准入后才能消费以提交。
pub struct RecordedRestore {
    instance: Arc<()>,
    request: Mutation,
    metadata: MutationMetadata,
    reference: store::RecordingRef,
    intent: ContentAuditEvent,
}
impl RecordedRestore {
    /// 事件在 prepare 时已固定；owner 只能投递该原事件。
    pub fn intent(&self) -> &ContentAuditEvent {
        &self.intent
    }
}
impl FsVaultBroker {
    /// 无安装副作用；正文必须来自同路径仍保留的历史，恢复 Intent 先可靠持久化。
    pub fn prepare_recorded_restore(
        &mut self,
        request: Mutation,
        metadata: MutationMetadata,
    ) -> Result<RecordedRestore, BrokerError> {
        mutation::validate_request(&request)?;
        crate::file_operation::guard_legacy_key(self, &sha256(request.idempotency_key.as_bytes()))?;
        crate::maintenance::ensure_idle(self)?;
        mutation::validate_request(&request)?;
        let MutationKind::Restore { source_revision } = metadata.kind else {
            return Err(BrokerError::IdempotencyConflict);
        };
        if !matches!(request.precondition, Precondition::Match(_)) {
            return Err(BrokerError::IdempotencyConflict);
        }
        let state = mutation::load(self)?;
        if self.writer.uncertain() || state.legacy_unresolved || state.operation.is_some() {
            return Err(BrokerError::MutationUnresolved);
        }
        super::pending::ensure_slot(
            self,
            &store::id(self, &sha256(request.idempotency_key.as_bytes())),
        )?;
        super::check_existing(self, &request, Some(&metadata))?;
        if let Some(previous) = self.mutation_replay(&request)? {
            let _ = previous;
            return Err(BrokerError::IdempotencyConflict);
        }
        if Revision::compute(self.vault_uuid, &request.path, &request.content) != source_revision {
            return Err(BrokerError::IdempotencyConflict);
        }
        let history = self.read_history(
            &request.path,
            source_revision,
            crate::writer::disk::CONTENT_LIMIT as u64,
        )?;
        if history.content != request.content {
            return Err(BrokerError::CorruptWriterState);
        }
        let envelope = store::prepare(self, &request, Some(&metadata))?;
        let row = events::row(&envelope, None)?;
        Ok(RecordedRestore {
            instance: Arc::clone(&self.capture_instance),
            request,
            metadata,
            reference: envelope.reference()?,
            intent: row.event,
        })
    }
    /// 仅在 owner 已验证 AuditRecord 的事件、完整性、offset 与 hash 后调用。
    pub fn confirm_recorded_restore_intent(
        &mut self,
        ticket: &RecordedRestore,
        offset: u64,
        record_hash: &str,
    ) -> Result<(), BrokerError> {
        let envelope = check(self, ticket)?;
        let row = events::row(&envelope, None)?;
        self.confirm_content_audit(
            delivery::candidate(self, &envelope, row)?,
            offset,
            record_hash,
        )
    }
    /// owner 最终重验会话/范围/根/epoch 后消费票据；CAS 与持久 Intent ACK 再次核验。
    pub fn commit_recorded_restore(
        &mut self,
        ticket: RecordedRestore,
    ) -> Result<MutationReceipt, BrokerError> {
        let envelope = check(self, &ticket)?;
        if !events::acknowledged(self, &envelope, &events::row(&envelope, None)?)? {
            return Err(BrokerError::MutationUnresolved);
        }
        mutation::apply_recorded(self, ticket.request, &ticket.metadata)
    }
    /// 最终准入失败时明确取消；已有安装阶段或已知成功从不被改写成取消。
    pub fn cancel_recorded_restore(&mut self, ticket: RecordedRestore) -> Result<(), BrokerError> {
        let envelope = check(self, &ticket)?;
        super::pending::cancel(self, &envelope)
    }
}
fn check(broker: &FsVaultBroker, ticket: &RecordedRestore) -> Result<store::Envelope, BrokerError> {
    if !Arc::ptr_eq(&broker.capture_instance, &ticket.instance) {
        return Err(BrokerError::CaptureChanged);
    }
    let envelope = store::load(broker, &ticket.reference)?;
    envelope.matches(&ticket.request, Some(&ticket.metadata))?;
    if events::row(&envelope, None)?.event != ticket.intent {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(envelope)
}
