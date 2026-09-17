//! sealed 票据只证明已核对字节；owner 的最后准入复验仍在本函数调用之前。
use super::PreparedStream;
use crate::blob::{current, file, store};
use crate::content_audit::{delivery, store as events};
use crate::mutation::record::{Outcome, Phase, Terminal};
use crate::writer::directory;
use crate::{BrokerError, FsVaultBroker, MutationReceipt, mutation};
use std::sync::Arc;

pub(super) fn check(
    broker: &FsVaultBroker,
    ticket: &PreparedStream,
) -> Result<(mutation::State, crate::recorded::store::Envelope), BrokerError> {
    crate::maintenance::ensure_idle(broker)?;
    if !Arc::ptr_eq(&broker.capture_instance, &ticket.instance) || broker.writer.uncertain() {
        return Err(BrokerError::CaptureChanged);
    }
    let state = mutation::load(broker)?;
    let op = state
        .operation
        .as_ref()
        .ok_or(BrokerError::CaptureChanged)?;
    if op.phase != Phase::Prepared
        || serde_json::to_vec(op).ok() != serde_json::to_vec(&ticket.operation).ok()
    {
        return Err(BrokerError::CaptureChanged);
    }
    if mutation::replay::load(&broker.data, &op.key_hash, state.pair_id)?.is_some() {
        return Err(BrokerError::CaptureChanged);
    }
    let envelope = crate::recorded::store::load(broker, &ticket.reference)?;
    if events::row(&envelope, None)?.event != ticket.intent {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok((state, envelope))
}
fn audit_ready(
    broker: &FsVaultBroker,
    envelope: &crate::recorded::store::Envelope,
) -> Result<(), BrokerError> {
    if matches!(envelope.kind, Some(crate::MutationKind::Restore { .. }))
        && !events::acknowledged(broker, envelope, &events::row(envelope, None)?)?
    {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(())
}
impl FsVaultBroker {
    /// 只确认本次恢复票据的原 Intent；回执内容与完整性由可信 owner 核对。
    pub fn confirm_stream_restore_intent(
        &mut self,
        ticket: &PreparedStream,
        offset: u64,
        record_hash: &str,
    ) -> Result<(), BrokerError> {
        let (_, envelope) = check(self, ticket)?;
        if !matches!(envelope.kind, Some(crate::MutationKind::Restore { .. })) {
            return Err(BrokerError::IdempotencyConflict);
        }
        let row = events::row(&envelope, None)?;
        self.confirm_content_audit(
            delivery::candidate(self, &envelope, row)?,
            offset,
            record_hash,
        )
    }
    /// 当前 canonical、sealed stage、每级路径和原意图不变才安装；Installing 后一律保留未知。
    pub fn commit_stream_mutation(
        &mut self,
        ticket: PreparedStream,
    ) -> Result<MutationReceipt, BrokerError> {
        let (mut state, envelope) = check(self, &ticket)?;
        audit_ready(self, &envelope)?;
        current::check(self, &ticket.operation.path, &ticket.current)?;
        let stages = directory(&self.root, ".workbench/staging")?;
        file::check(
            &stages,
            &store::logical(&ticket.operation.stage.name)?,
            &ticket.stage.node,
        )?;
        #[cfg(test)]
        crate::blob::install::before_history_check_test(self, &ticket.operation, false)?;
        // L0 漂移在进入 Installing 前失败，不能先覆盖 canonical 再由索引收尾发现。
        ticket.history.check(self, &ticket.operation)?;
        let op = state
            .operation
            .as_mut()
            .ok_or(BrokerError::CaptureChanged)?;
        op.phase = Phase::Installing;
        mutation::save(self, &state)?;
        mutation::fault(self, 2).map_err(|_| BrokerError::MutationUnresolved)?;
        crate::blob::install::sealed(
            self,
            &ticket.operation,
            &ticket.current,
            &ticket.stage,
            &ticket.history,
        )
        .map_err(|_| BrokerError::MutationUnresolved)?;
        mutation::fault(self, 3).map_err(|_| BrokerError::MutationUnresolved)?;
        let op = state
            .operation
            .as_mut()
            .ok_or(BrokerError::MutationUnresolved)?;
        op.phase = Phase::Completed;
        op.receipt = Some(op.receipt());
        let receipt = op.receipt();
        mutation::save(self, &state).map_err(|_| BrokerError::MutationUnresolved)?;
        mutation::fault(self, 4).map_err(|_| BrokerError::MutationUnresolved)?;
        mutation::finish(self, &mut state).map_err(|_| BrokerError::MutationUnresolved)?;
        Ok(receipt)
    }
    /// 只取消同实例未安装的 Prepared 原请求；严格恢复先核对原 Intent ACK。
    pub fn cancel_stream_mutation(&mut self, ticket: PreparedStream) -> Result<(), BrokerError> {
        let (mut state, envelope) = check(self, &ticket)?;
        audit_ready(self, &envelope)?;
        // 不读取漂移后的 canonical；取消仅证明本 Prepared 尚未进入安装。
        mutation::install::verify_stage(self, &ticket.operation)?;
        let terminal = Terminal {
            format_version: 4,
            recording: Some(ticket.reference),
            pair_id: state.pair_id,
            sequence: None,
            fingerprint: envelope.fingerprint,
            expected_revision: envelope.target,
            outcome: Outcome::Cancelled {
                path: envelope.path,
                precondition: envelope.precondition,
            },
        };
        mutation::save_terminal(self, &envelope.key_hash, &terminal)?;
        self.capture_instance = Arc::new(());
        mutation::finish(self, &mut state)
    }
}
