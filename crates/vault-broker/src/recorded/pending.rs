//! 浏览器丢失请求键之后仍可显式取消未进入安装的持久恢复 Intent。

use super::store::{self, Envelope};
use crate::content_audit::store as events;
use crate::mutation::record::{Outcome, Terminal};
use crate::writer::disk;
use crate::{BrokerError, ContentAuditEvent, FsVaultBroker, MutationKind, mutation};
use std::sync::Arc;

/// 尚未进入安装的持久恢复意图审阅票据，仅供同实例 owner 明确取消。
pub struct PendingRecordedRestore {
    instance: Arc<()>,
    journal: disk::FilePin,
    reference: store::RecordingRef,
    intent: ContentAuditEvent,
}
impl PendingRecordedRestore {
    /// 固定来源、路径和 revision；不暴露原 key 或正文。
    pub fn intent(&self) -> &ContentAuditEvent {
        &self.intent
    }
}
impl FsVaultBroker {
    /// 安装阶段另走 pending_mutation；此列表只提供尚未进入安装的可取消审阅。
    pub fn pending_recorded_restores(
        &self,
        limit: usize,
    ) -> Result<Vec<PendingRecordedRestore>, BrokerError> {
        if !(1..=64).contains(&limit) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let state = mutation::load(self)?;
        if self.writer.uncertain() || state.legacy_unresolved {
            return Err(BrokerError::MutationUnresolved);
        }
        let names = events::entries(self)?;
        let mut result = Vec::new();
        for filename in &names {
            let (envelope, row) = events::lifecycle(self, filename)?;
            if !matches!(envelope.kind, Some(MutationKind::Restore { .. }))
                || events::terminal(self, &envelope)?.is_some()
                || state
                    .operation
                    .as_ref()
                    .is_some_and(|op| op.key_hash == envelope.key_hash)
            {
                continue;
            }
            if result.len() < limit {
                result.push(PendingRecordedRestore {
                    instance: Arc::clone(&self.capture_instance),
                    journal: disk::FilePin::capture(
                        &self.data,
                        "writer-state.json",
                        disk::RECORD_LIMIT,
                    )?,
                    reference: envelope.reference()?,
                    intent: row.event,
                });
            }
        }
        if events::entries(self)? != names {
            return Err(BrokerError::CaptureChanged);
        }
        Ok(result)
    }
    /// owner 完成受信审阅后明确取消；重开不会自动取消，更不会凭当前 hash 补成功。
    pub fn cancel_pending_recorded_restore(
        &mut self,
        review: PendingRecordedRestore,
    ) -> Result<(), BrokerError> {
        crate::maintenance::ensure_idle(self)?;
        if !Arc::ptr_eq(&self.capture_instance, &review.instance) {
            return Err(BrokerError::CaptureChanged);
        }
        review
            .journal
            .check(&self.data, "writer-state.json", disk::RECORD_LIMIT)?;
        let envelope = store::load(self, &review.reference)?;
        if events::row(&envelope, None)?.event != review.intent {
            return Err(BrokerError::CorruptWriterState);
        }
        cancel(self, &envelope)
    }
}
pub(super) fn cancel(broker: &FsVaultBroker, envelope: &Envelope) -> Result<(), BrokerError> {
    crate::maintenance::ensure_idle(broker)?;
    cancel_for_maintenance(broker, envelope)
}
pub(crate) fn cancel_for_maintenance(
    broker: &FsVaultBroker,
    envelope: &Envelope,
) -> Result<(), BrokerError> {
    let state = mutation::load(broker)?;
    if broker.writer.uncertain() || state.legacy_unresolved || state.operation.is_some() {
        return Err(BrokerError::MutationUnresolved);
    }
    if !matches!(envelope.kind, Some(MutationKind::Restore { .. })) {
        return Err(BrokerError::CorruptWriterState);
    }
    if let Some(terminal) = events::terminal(broker, envelope)? {
        return if matches!(terminal.outcome, Outcome::Cancelled { .. }) {
            Ok(())
        } else {
            Err(BrokerError::IdempotencyConflict)
        };
    }
    events::lifecycle(
        broker,
        &events::name(envelope, crate::ContentAuditPhase::Intent),
    )?;
    let terminal = Terminal {
        format_version: 4,
        recording: Some(envelope.reference()?),
        pair_id: state.pair_id,
        sequence: None,
        fingerprint: envelope.fingerprint.clone(),
        expected_revision: envelope.target,
        outcome: Outcome::Cancelled {
            path: envelope.path.clone(),
            precondition: envelope.precondition,
        },
    };
    mutation::save_terminal(broker, &envelope.key_hash, &terminal)
}

pub(crate) fn ensure_slot(broker: &FsVaultBroker, id: &str) -> Result<(), BrokerError> {
    for filename in events::entries(broker)? {
        let (envelope, _) = events::lifecycle(broker, &filename)?;
        if envelope.id == id || !matches!(envelope.kind, Some(MutationKind::Restore { .. })) {
            continue;
        }
        let outcome = events::outcome_row(broker, &envelope)?;
        let harmless = outcome.as_ref().is_some_and(|row| {
            matches!(
                row.event.outcome,
                Some(crate::ContentAuditOutcome::Rejected | crate::ContentAuditOutcome::Cancelled)
            )
        });
        let ack = outcome
            .as_ref()
            .map(|row| events::acknowledged(broker, &envelope, row))
            .transpose()?
            .unwrap_or(false);
        if !harmless && !ack {
            return Err(BrokerError::MutationUnresolved);
        }
    }
    Ok(())
}
