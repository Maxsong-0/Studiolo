//! 固定事件只由不可变请求描述符和持久终态派生。
use super::{ContentAuditEvent, ContentAuditOutcome, ContentAuditPhase};
use crate::mutation::{
    self,
    record::{Outcome, Terminal},
    replay::Replay,
};
use crate::recorded::store::{Envelope, RecordingRef};
use crate::{BrokerError, FsVaultBroker, Precondition, VaultUuid, sha256, writer::disk};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Row {
    pub version: u32,
    pub pair_id: VaultUuid,
    pub recording: RecordingRef,
    pub event: ContentAuditEvent,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ack {
    pub version: u32,
    pub pair_id: VaultUuid,
    pub event_id: String,
    pub phase: ContentAuditPhase,
    pub digest: String,
    pub offset: u64,
    pub record_hash: String,
}
pub(crate) fn name(envelope: &Envelope, phase: ContentAuditPhase) -> String {
    let suffix = match phase {
        ContentAuditPhase::Intent => "intent",
        ContentAuditPhase::Outcome => "outcome",
    };
    format!("{}-{suffix}.json", envelope.id)
}
pub(crate) fn digest(row: &Row) -> Result<String, BrokerError> {
    Ok(sha256(
        &serde_json::to_vec(row).map_err(|_| BrokerError::CorruptWriterState)?,
    ))
}
pub(crate) fn row(
    envelope: &Envelope,
    outcome: Option<ContentAuditOutcome>,
) -> Result<Row, BrokerError> {
    let event = ContentAuditEvent {
        event_id: format!("content-{}", envelope.id),
        phase: if outcome.is_some() {
            ContentAuditPhase::Outcome
        } else {
            ContentAuditPhase::Intent
        },
        vault_uuid: envelope.vault_uuid,
        path: envelope.path.clone(),
        kind: envelope
            .kind
            .clone()
            .ok_or(BrokerError::CorruptWriterState)?,
        actor: envelope
            .actor
            .clone()
            .ok_or(BrokerError::CorruptWriterState)?,
        requested_at_ms: envelope.requested_at_ms,
        previous_revision: match envelope.precondition {
            Precondition::Match(value) => Some(value),
            Precondition::MustNotExist => None,
        },
        new_revision: envelope.target,
        sha256: envelope.sha256.clone(),
        bytes: envelope.bytes,
        outcome,
    };
    Ok(Row {
        version: 1,
        pair_id: envelope.pair_id,
        recording: envelope.reference()?,
        event,
    })
}
pub(crate) fn outcome(terminal: &Terminal) -> ContentAuditOutcome {
    match &terminal.outcome {
        Outcome::Committed { .. } => ContentAuditOutcome::Committed,
        Outcome::Rejected { .. } => ContentAuditOutcome::Rejected,
        Outcome::Cancelled { .. } => ContentAuditOutcome::Cancelled,
        Outcome::Indeterminate { .. } => ContentAuditOutcome::Indeterminate,
    }
}
pub(crate) fn terminal(
    broker: &FsVaultBroker,
    envelope: &Envelope,
) -> Result<Option<Terminal>, BrokerError> {
    let Some(replay) = mutation::replay::load(&broker.data, &envelope.key_hash, envelope.pair_id)?
    else {
        return Ok(None);
    };
    let Replay::Current(terminal) = replay else {
        return Err(BrokerError::CorruptWriterState);
    };
    if terminal.recording.as_ref() != Some(&envelope.reference()?) {
        return Err(BrokerError::CorruptWriterState);
    }
    crate::recorded::validate_terminal(broker, &terminal)?;
    Ok(Some(terminal))
}
pub(crate) fn outcome_row(
    broker: &FsVaultBroker,
    envelope: &Envelope,
) -> Result<Option<Row>, BrokerError> {
    terminal(broker, envelope)?
        .as_ref()
        .map(|value| row(envelope, Some(outcome(value))))
        .transpose()
}
pub(crate) fn validate_ack(
    broker: &FsVaultBroker,
    row: &Row,
    ack: &Ack,
) -> Result<(), BrokerError> {
    if ack.version != 1
        || ack.pair_id != broker.writer.binding.pair_id
        || ack.event_id != row.event.event_id
        || ack.phase != row.event.phase
        || ack.digest != digest(row)?
        || ack.offset == 0
        || !disk::hex(&ack.record_hash)
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(())
}
