//! 固定正文审计映射；临时HTTP故障不制造永久操作Outcome。
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditLedger, AuditPhase, SanitizedPayload};
use studiolo_vault_broker::{
    ContentAuditEvent, ContentAuditOutcome, ContentAuditPhase, MutationKind,
};

/// 只接受Broker已固定的事件，完整核对返回回执后才交给owner确认。
pub(crate) fn append(
    ledger: &dyn AuditLedger,
    value: &ContentAuditEvent,
) -> Result<(u64, String), ()> {
    let event = event(value)?;
    let record = ledger.append(event.clone()).map_err(|_| ())?;
    if record.event() != &event || record.verify_integrity().is_err() {
        return Err(());
    }
    Ok((record.offset(), record.record_hash().to_hex()))
}

fn event(value: &ContentAuditEvent) -> Result<AuditEvent, ()> {
    let phase = match value.phase {
        ContentAuditPhase::Intent => AuditPhase::Intent,
        ContentAuditPhase::Outcome => AuditPhase::Outcome,
    };
    let (action, source) = match value.kind {
        MutationKind::Replace => ("vault.mutation", None),
        MutationKind::Restore { source_revision } => {
            ("recovery.page_restore", Some(source_revision.to_hex()))
        }
    };
    let outcome = value.outcome.map(|outcome| match outcome {
        ContentAuditOutcome::Committed => "committed",
        ContentAuditOutcome::Rejected => "rejected",
        ContentAuditOutcome::Cancelled => "cancelled",
        ContentAuditOutcome::Indeterminate => "indeterminate",
    });
    let committed = value.outcome == Some(ContentAuditOutcome::Committed);
    let payload = serde_json::json!({"schema_version":1,"actor":value.actor,
        "vault_uuid":value.vault_uuid.to_string(),"path":value.path.to_string(),
        "requested_at_ms":value.requested_at_ms,"source_revision":source,
        "expected_revision":value.previous_revision.map(|revision| revision.to_hex()),
        "proposed_revision":value.new_revision.to_hex(),
        "new_revision":committed.then(|| value.new_revision.to_hex()),
        "sha256":value.sha256,"bytes":value.bytes,"outcome":outcome});
    Ok(AuditEvent::new(
        AuditEventId::parse(&value.event_id).map_err(|_| ())?,
        phase,
        action,
        SanitizedPayload::try_from(payload).map_err(|_| ())?,
    ))
}
