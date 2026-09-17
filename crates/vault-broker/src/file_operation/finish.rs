//! 重开只投影主日志已有的固定事实；Issued 从不凭当前缺失/同 hash 自动转成功。
use super::*;
use crate::{BrokerError, FsVaultBroker, mutation};

pub(crate) fn set_terminal(
    state: &mut mutation::State,
    outcome: FileOperationOutcome,
    evidence: Option<String>,
) -> Result<(), BrokerError> {
    let active = state
        .file_operation
        .as_mut()
        .ok_or(BrokerError::CorruptWriterState)?;
    if active.rename.is_some() {
        return super::rename_terminal::set(state, outcome, evidence);
    }
    let before = active.before.presence()?;
    let effect = match outcome {
        FileOperationOutcome::Committed => FileEffect::Confirmed {
            path: active.frozen.path.clone(),
            before: before.clone(),
            after: active
                .executed
                .as_ref()
                .ok_or(BrokerError::CorruptWriterState)?
                .presence()?,
        },
        FileOperationOutcome::Rejected | FileOperationOutcome::Cancelled => {
            FileEffect::NotApplied {
                path: active.frozen.path.clone(),
            }
        }
        FileOperationOutcome::Indeterminate => FileEffect::Unknown {
            path: active.frozen.path.clone(),
        },
    };
    let tombstone_id = (outcome == FileOperationOutcome::Committed
        && matches!(active.frozen.kind, FileOperationKind::Delete { .. }))
    .then(|| active.frozen.operation_id.clone());
    active.terminal = Some(record::Terminal {
        format_version: 5,
        rename: None,
        frozen: active.frozen.clone(),
        observed_before: before,
        effect_at_ms: active.effect_at_ms,
        receipt: FileOperationReceipt {
            operation_id: active.frozen.operation_id.clone(),
            sequence: active.sequence,
            outcome,
            effects: vec![effect],
            tombstone_id,
            evidence_id: evidence,
        },
    });
    active.phase = FileOperationPhase::FinalizedAwaitAudit;
    if outcome == FileOperationOutcome::Committed {
        state.committed_through = active.sequence;
    }
    Ok(())
}
pub(crate) fn resume(
    broker: &FsVaultBroker,
    state: &mut mutation::State,
) -> Result<(), BrokerError> {
    let Some(active) = &state.file_operation else {
        return Ok(());
    };
    let request = request::ensure(broker, active)?;
    if active.rejected
        && matches!(
            active.phase,
            FileOperationPhase::Preparing | FileOperationPhase::Prepared
        )
        && audit::confirmed(broker, &audit::intent(&request))?
    {
        set_terminal(state, FileOperationOutcome::Rejected, None)?;
        mutation::save(broker, state)?;
    }
    let active = state
        .file_operation
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    if matches!(
        active.phase,
        FileOperationPhase::Detached
            | FileOperationPhase::Installed
            | FileOperationPhase::SourceDetached
    ) {
        set_terminal(state, FileOperationOutcome::Committed, None)?;
        mutation::save(broker, state)?;
    }
    let active = state
        .file_operation
        .as_ref()
        .ok_or(BrokerError::CorruptWriterState)?;
    let Some(terminal) = &active.terminal else {
        return Ok(());
    };
    terminal::save(broker, terminal)?;
    if terminal.receipt.outcome == FileOperationOutcome::Committed {
        if matches!(
            active.frozen.kind,
            FileOperationKind::Delete { .. } | FileOperationKind::Rename { .. }
        ) {
            tombstone::save(broker, active)?;
        }
        crate::history::complete_file_operation(broker, active)?;
    }
    crate::mutation::fault(broker, 58)?;
    let request = request::ensure(broker, active)?;
    if audit::confirmed(broker, &audit::outcome(&request, terminal)?)? {
        if !audit::confirmed(broker, &audit::intent(&request))? {
            return Err(BrokerError::CorruptWriterState);
        }
        // 所有实际效果和固定结果已持久；ACK 不改变 committed_through。
        state.file_operation = None;
        mutation::save(broker, state)?;
    }
    Ok(())
}
