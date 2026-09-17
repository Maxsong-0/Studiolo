//! owner 最终 Auth/scope 复验后借用固定票据；拒绝固定、Issued 未知、终态不重新安装。
use super::*;
use crate::{BrokerError, FsVaultBroker, mutation};
impl FsVaultBroker {
    /// 只有真实 Intent 已确认才可能执行；借用失败保留票据，重复借用只重放已知终态。
    /// 结果返回后仍须投递并确认固定 Outcome；Issued 不允许重新执行。
    pub fn commit_file_operation(
        &mut self,
        ticket: &FileOperationTicket,
    ) -> Result<FileOperationReceipt, BrokerError> {
        crate::maintenance::ensure_no_maintenance(self)?;
        let mut state = ticket::check(self, ticket)?;
        let request = request::load(self, &ticket.intent.operation_id)?
            .ok_or(BrokerError::CorruptWriterState)?;
        if let Some(terminal) = terminal::load(self, &request.frozen)? {
            return Ok(terminal.receipt);
        }
        if self.content_audit_status()?.restore_unconfirmed {
            return Err(BrokerError::MutationUnresolved);
        }
        if self.writer.uncertain() {
            return Err(BrokerError::MutationUnresolved);
        }
        if !audit::confirmed(self, &ticket.intent)? {
            return Err(BrokerError::MutationUnresolved);
        }
        let active = state
            .file_operation
            .as_mut()
            .ok_or(BrokerError::CaptureChanged)?;
        if active.phase != FileOperationPhase::Prepared {
            return Err(BrokerError::MutationUnresolved);
        }
        if active.rejected {
            finish::set_terminal(&mut state, FileOperationOutcome::Rejected, None)?;
            mutation::save(self, &state)?;
        } else if active.rename.is_some() {
            super::rename_commit::execute(self, &mut state)?;
        } else {
            prepared::check(self, active)?;
            capacity::free(
                self,
                active
                    .history
                    .as_ref()
                    .ok_or(BrokerError::CorruptWriterState)?
                    .identity
                    .bytes
                    + crate::blob::OBJECT_LIMIT * 3,
            )?;
            active.phase = match active.frozen.kind {
                FileOperationKind::Rename { .. } => return Err(BrokerError::CorruptWriterState),
                FileOperationKind::Delete { .. } => FileOperationPhase::DetachIssued,
                FileOperationKind::RestoreTombstone { .. } => FileOperationPhase::InstallIssued,
            };
            mutation::save(self, &state)?;
            let result = (|| {
                mutation::fault(self, 52)?;
                install::execute(
                    self,
                    state
                        .file_operation
                        .as_mut()
                        .ok_or(BrokerError::CorruptWriterState)?,
                )?;
                mutation::save(self, &state)?;
                mutation::fault(self, 55)
            })();
            if result.is_err() {
                return Err(BrokerError::MutationUnresolved);
            }
        }
        finish::resume(self, &mut state).map_err(|_| BrokerError::MutationUnresolved)?;
        terminal::load(self, &request.frozen)?
            .map(|value| value.receipt)
            .ok_or(BrokerError::MutationUnresolved)
    }
    /// 同实例、尚未 issued 的票据可以明确取消；严格 Intent 仍必须先有真实 ACK。
    pub fn cancel_file_operation(
        &mut self,
        ticket: FileOperationTicket,
    ) -> Result<FileOperationReceipt, BrokerError> {
        crate::maintenance::ensure_no_maintenance(self)?;
        let mut state = ticket::check(self, &ticket)?;
        if self.writer.uncertain() {
            return Err(BrokerError::MutationUnresolved);
        }
        let request = request::load(self, &ticket.intent.operation_id)?
            .ok_or(BrokerError::CorruptWriterState)?;
        // 旧终态票据只返回原回执，绝不能把当前另一个活动槽当作取消对象。
        if let Some(terminal) = terminal::load(self, &request.frozen)? {
            return Ok(terminal.receipt);
        }
        let active = state
            .file_operation
            .as_ref()
            .ok_or(BrokerError::CaptureChanged)?;
        if active.sequence != ticket.sequence
            || !active.frozen.matches(&request.frozen)
            || active.frozen.operation_id != ticket.intent.operation_id
        {
            return Err(BrokerError::CaptureChanged);
        }
        if !audit::confirmed(self, &ticket.intent)? {
            return Err(BrokerError::MutationUnresolved);
        }
        if !matches!(
            active.phase,
            FileOperationPhase::Preparing | FileOperationPhase::Prepared
        ) {
            return Err(BrokerError::MutationUnresolved);
        }
        let frozen = active.frozen.clone();
        let result = if active.rejected {
            FileOperationOutcome::Rejected
        } else {
            FileOperationOutcome::Cancelled
        };
        finish::set_terminal(&mut state, result, None)?;
        mutation::save(self, &state)?;
        finish::resume(self, &mut state)?;
        terminal::load(self, &frozen)?
            .map(|value| value.receipt)
            .ok_or(BrokerError::MutationUnresolved)
    }
}
