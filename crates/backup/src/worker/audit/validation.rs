//! 重新生成唯一合法事件并做完整比较；只检查 action 名称远远不够。
use super::{AuditMessageError, Result};
use crate::worker::{
    job::{BACKUP_AUDIT_ACTION, backup_intent, payload},
    recovery_point::BackupTargetKind,
    restic::SnapshotRef,
    state::JobKey,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};
use studiolo_core::checkpoint::InstantMillis;

pub(super) fn event(event: &AuditEvent, key: &JobKey, target: BackupTargetKind) -> Result<()> {
    let expected = match event.phase() {
        AuditPhase::Intent => backup_intent(key, target).map_err(|_| AuditMessageError::Invalid)?,
        AuditPhase::Outcome => {
            let value = event.payload().as_value();
            let body = match value.get("result").and_then(Value::as_str) {
                Some("failed") => payload::outcome_failed(key, field(value, "reason")?),
                Some("unverified") => payload::outcome_unverified(key, field(value, "reason")?),
                Some("succeeded") => success(key, value)?,
                _ => return Err(AuditMessageError::Invalid),
            };
            AuditEvent::new(
                AuditEventId::parse(payload::event_id_text(key))
                    .map_err(|_| AuditMessageError::Invalid)?,
                AuditPhase::Outcome,
                BACKUP_AUDIT_ACTION,
                SanitizedPayload::try_from(body).map_err(|_| AuditMessageError::Invalid)?,
            )
        }
    };
    if event == &expected {
        Ok(())
    } else {
        Err(AuditMessageError::Invalid)
    }
}
fn success(key: &JobKey, value: &Value) -> Result<Value> {
    let data: SnapshotRef = field(value, "data_snapshot")?;
    let final_snapshot: SnapshotRef = field(value, "final_snapshot")?;
    let verified: InstantMillis = field(value, "data_verified_at")?;
    let finalized: InstantMillis = field(value, "finalized_at")?;
    if data.as_str().len() != 64
        || final_snapshot.as_str().len() != 64
        || data == final_snapshot
        || verified.millis() < 0
        || finalized < verified
    {
        return Err(AuditMessageError::Invalid);
    }
    Ok(payload::outcome_success_fields(
        key,
        &data,
        &final_snapshot,
        verified,
        finalized,
    ))
}
fn field<T: DeserializeOwned>(value: &Value, name: &str) -> Result<T> {
    serde_json::from_value(value.get(name).ok_or(AuditMessageError::Invalid)?.clone())
        .map_err(|_| AuditMessageError::Invalid)
}
