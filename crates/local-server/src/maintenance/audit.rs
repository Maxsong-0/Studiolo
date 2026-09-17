//! 维护审计保留原请求身份；本机操作者只进入独立 writer.maintenance 事件。

use super::MaintenanceError;
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditLedger, AuditPhase, SanitizedPayload};
use studiolo_vault_broker::{
    ContentAuditPhase, MaintenanceAuditEvent, MaintenanceCurrent, MaintenanceDecision,
    MaintenanceOutcome, MaintenanceTargetKind, ReconciliationOutcome,
};

pub(super) fn append(
    ledger: &dyn AuditLedger,
    value: &MaintenanceAuditEvent,
) -> Result<(u64, String), MaintenanceError> {
    let event = event(value)?;
    let record = ledger
        .append(event.clone())
        .map_err(|_| MaintenanceError::Audit)?;
    if record.event() != &event || record.verify_integrity().is_err() {
        return Err(MaintenanceError::Audit);
    }
    Ok((record.offset(), record.record_hash().to_hex()))
}

fn event(value: &MaintenanceAuditEvent) -> Result<AuditEvent, MaintenanceError> {
    let schema_version = schema(value)?;
    let phase = match value.phase {
        ContentAuditPhase::Intent => AuditPhase::Intent,
        ContentAuditPhase::Outcome => AuditPhase::Outcome,
    };
    let payload = serde_json::json!({
        "schema_version": schema_version,
        "actor": value.actor,
        "decision": value.decision,
        "requested_at_ms": value.requested_at_ms,
        "vault_uuid": value.vault_uuid.to_string(),
        "target": value.target,
        "outcome": value.outcome,
        "resolution": value.resolution,
        "evidence_id": value.evidence_id,
    });
    Ok(AuditEvent::new(
        AuditEventId::parse(&value.event_id).map_err(|_| MaintenanceError::Audit)?,
        phase,
        "writer.maintenance",
        SanitizedPayload::try_from(payload).map_err(|_| MaintenanceError::Audit)?,
    ))
}

fn schema(value: &MaintenanceAuditEvent) -> Result<u32, MaintenanceError> {
    let target = &value.target;
    let version = match (value.schema_version, target.file_operation_id.as_deref()) {
        (None, None)
            if target.secondary.is_none()
                && target.file_operation.is_none()
                && target.target_revision.is_some()
                && !matches!(target.current, MaintenanceCurrent::Unobservable { .. }) =>
        {
            1
        }
        (Some(2), Some(id))
            if target.secondary.is_none()
                && target.file_operation.is_none()
                && hex(id)
                && target.sequence.is_some()
                && target.kind != MaintenanceTargetKind::RestoreIntent =>
        {
            2
        }
        (Some(3), Some(id))
            if hex(id) && target.sequence.is_some() && super::rename::valid(target) =>
        {
            3
        }
        _ => return Err(MaintenanceError::Audit),
    };
    if !hex(&value.event_id)
        || !hex(&target.review_id)
        || !hex(&target.request_fingerprint)
        || target.sequence == Some(0)
        || value.requested_at_ms.is_some_and(|time| time < 0)
        || !matches!(
            (target.kind, value.decision),
            (
                MaintenanceTargetKind::Prepared,
                MaintenanceDecision::CancelPrepared
            ) | (
                MaintenanceTargetKind::Installing,
                MaintenanceDecision::PreserveInstalling
            ) | (
                MaintenanceTargetKind::RestoreIntent,
                MaintenanceDecision::CancelRestoreIntent
            )
        )
    {
        return Err(MaintenanceError::Audit);
    }
    let valid = match (value.phase, value.outcome) {
        (ContentAuditPhase::Intent, None) => {
            value.resolution.is_none() && value.evidence_id.is_none()
        }
        (ContentAuditPhase::Outcome, Some(MaintenanceOutcome::Applied)) => {
            if value.decision == MaintenanceDecision::PreserveInstalling {
                value.resolution == Some(ReconciliationOutcome::Indeterminate)
                    && value.evidence_id.as_deref().is_some_and(hex)
            } else {
                value.resolution == Some(ReconciliationOutcome::Cancelled)
                    && value.evidence_id.is_none()
            }
        }
        (
            ContentAuditPhase::Outcome,
            Some(MaintenanceOutcome::Cancelled | MaintenanceOutcome::NotApplied),
        ) => value.resolution.is_none() && value.evidence_id.is_none(),
        _ => false,
    };
    if valid {
        Ok(version)
    } else {
        Err(MaintenanceError::Audit)
    }
}

fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
