//! 票据只适用于原实例与精确 Preparing/Prepared 请求；终态重放不观察当前正文。
use super::*;
use crate::{BrokerError, FsVaultBroker, mutation};
use std::sync::Arc;

pub(crate) fn from_active(
    broker: &FsVaultBroker,
    active: &record::Active,
) -> Result<FileOperationTicket, BrokerError> {
    let request = request::ensure(broker, active)?;
    Ok(FileOperationTicket {
        instance: Arc::clone(&broker.capture_instance),
        sequence: active.sequence,
        fingerprint: active.frozen.fingerprint.clone(),
        preview: preview(
            &request,
            active.rejected.then_some(FileOperationOutcome::Rejected),
        ),
        intent: audit::intent(&request),
    })
}
pub(crate) fn from_terminal(
    broker: &FsVaultBroker,
    terminal: &record::Terminal,
) -> Result<FileOperationTicket, BrokerError> {
    let request = request::load(broker, &terminal.frozen.operation_id)?
        .ok_or(BrokerError::CorruptWriterState)?;
    audit::outcome(&request, terminal)?;
    Ok(FileOperationTicket {
        instance: Arc::clone(&broker.capture_instance),
        sequence: request.sequence,
        fingerprint: request.frozen.fingerprint.clone(),
        preview: preview(&request, Some(terminal.receipt.outcome)),
        intent: audit::intent(&request),
    })
}
pub(crate) fn preview(
    request: &request::Request,
    outcome: Option<FileOperationOutcome>,
) -> FileOperationPreview {
    FileOperationPreview {
        operation_id: request.frozen.operation_id.clone(),
        path: request.frozen.path.clone(),
        kind: request.frozen.kind.clone(),
        observed_before: request.observed_before.clone(),
        fixed_outcome: outcome,
    }
}
pub(crate) fn check(
    broker: &FsVaultBroker,
    ticket: &FileOperationTicket,
) -> Result<mutation::State, BrokerError> {
    if !Arc::ptr_eq(&broker.capture_instance, &ticket.instance) {
        return Err(BrokerError::CaptureChanged);
    }
    let state = mutation::load(broker)?;
    let request =
        request::load(broker, &ticket.intent.operation_id)?.ok_or(BrokerError::CaptureChanged)?;
    if request.sequence != ticket.sequence
        || request.frozen.fingerprint != ticket.fingerprint
        || audit::intent(&request) != ticket.intent
    {
        return Err(BrokerError::CaptureChanged);
    }
    if terminal::load(broker, &request.frozen)?.is_some() {
        return Ok(state);
    }
    if let Some(active) = &state.file_operation {
        if !active.frozen.matches(&request.frozen) || active.sequence != request.sequence {
            return Err(BrokerError::CaptureChanged);
        }
    } else if terminal::load(broker, &request.frozen)?.is_none() {
        return Err(BrokerError::CaptureChanged);
    }
    Ok(state)
}
