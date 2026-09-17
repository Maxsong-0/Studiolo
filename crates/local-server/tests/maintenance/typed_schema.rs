use super::super::{audit, presentation};
use serde_json::{Value, json};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, MemoryAuditLedger};
use studiolo_vault_broker::{
    ContentAuditPhase, MaintenanceAuditEvent, MaintenanceCurrent, MaintenanceDecision,
    MaintenanceOutcome, MaintenanceTargetKind, MaintenanceUnavailableReason, ReconciliationOutcome,
};

fn old_json() -> Value {
    json!({
        "event_id":"1".repeat(64),"phase":"intent",
        "vault_uuid":"57c07e80-49f2-4493-9268-a409d473701a",
        "actor":{"kind":"local_os","uid":501,"gid":20},
        "decision":"cancel_prepared","requested_at_ms":null,
        "target":{
            "review_id":"2".repeat(64),"request_fingerprint":"3".repeat(64),
            "path":"note.md","kind":"prepared","sequence":1,
            "previous_revision":null,"target_revision":vec![4;32],
            "current":{"kind":"missing"},"proposed_available":true
        },
        "outcome":null,"resolution":null,"evidence_id":null
    })
}

fn typed() -> MaintenanceAuditEvent {
    let mut value: MaintenanceAuditEvent = serde_json::from_value(old_json()).unwrap();
    value.schema_version = Some(2);
    value.target.file_operation_id = Some("5".repeat(64));
    value.target.target_revision = None;
    value.target.current = MaintenanceCurrent::NotObserved;
    value
}

#[test]
fn legacy_json_and_actual_audit_schema_one_bytes_remain_exact() {
    let old = old_json();
    let event: MaintenanceAuditEvent = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(
        serde_json::to_vec(&serde_json::to_value(&event).unwrap()).unwrap(),
        serde_json::to_vec(&old).unwrap()
    );
    assert!(event.schema_version.is_none());
    assert!(event.target.file_operation_id.is_none());
    assert!(event.target.target_revision.is_some());
    let expected = json!({
        "schema_version":1,"actor":old["actor"],"decision":old["decision"],
        "requested_at_ms":old["requested_at_ms"],"vault_uuid":old["vault_uuid"],
        "target":old["target"],"outcome":null,"resolution":null,"evidence_id":null
    });
    let ledger = MemoryAuditLedger::new();
    let first = audit::append(&ledger, &event).unwrap();
    assert_eq!(audit::append(&ledger, &event).unwrap(), first);
    let records = ledger.query(&AuditFilter::default()).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(
        serde_json::to_vec(records[0].event().payload().as_value()).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
}

#[test]
fn typed_null_revision_and_unobservable_keep_their_schema_two_meanings() {
    for reason in [
        MaintenanceUnavailableReason::UnsafeOrUnstable,
        MaintenanceUnavailableReason::LimitExceeded,
        MaintenanceUnavailableReason::IoUnavailable,
    ] {
        let mut value = typed();
        value.decision = MaintenanceDecision::PreserveInstalling;
        value.target.kind = MaintenanceTargetKind::Installing;
        value.target.current = MaintenanceCurrent::Unobservable { reason };
        let ledger = MemoryAuditLedger::new();
        audit::append(&ledger, &value).unwrap();
        let rows = ledger.query(&AuditFilter::default()).unwrap();
        let payload = rows[0].event().payload().as_value();
        assert_eq!(payload["schema_version"], 2);
        assert!(payload["target"]["target_revision"].is_null());
        assert_eq!(payload["target"]["current"]["kind"], "unobservable");
        assert_eq!(
            payload["target"]["current"]["reason"],
            serde_json::to_value(reason).unwrap()
        );
        assert!(presentation::current(value.target.current).contains("不是缺失"));
    }
    assert!(presentation::current(MaintenanceCurrent::Missing).starts_with("已观察为缺失"));
    assert!(presentation::current(MaintenanceCurrent::NotObserved).starts_with("未观察"));
}

#[test]
fn invalid_version_target_and_outcome_combinations_are_not_acknowledged() {
    let mut invalid = Vec::new();
    let mut value = typed();
    value.schema_version = None;
    invalid.push(value);
    let mut value = typed();
    value.schema_version = Some(1);
    invalid.push(value);
    let mut value = typed();
    value.schema_version = Some(3);
    invalid.push(value);
    let mut value = typed();
    value.target.file_operation_id = None;
    invalid.push(value);
    let mut value = typed();
    value.target.sequence = None;
    invalid.push(value);
    let mut value = typed();
    value.phase = ContentAuditPhase::Outcome;
    invalid.push(value);
    let mut value = typed();
    value.outcome = Some(MaintenanceOutcome::Applied);
    invalid.push(value);
    let mut value = typed();
    value.resolution = Some(ReconciliationOutcome::Cancelled);
    invalid.push(value);
    let mut value = typed();
    value.phase = ContentAuditPhase::Outcome;
    value.outcome = Some(MaintenanceOutcome::Applied);
    invalid.push(value);
    let mut value: MaintenanceAuditEvent = serde_json::from_value(old_json()).unwrap();
    value.target.current = MaintenanceCurrent::Unobservable {
        reason: MaintenanceUnavailableReason::IoUnavailable,
    };
    invalid.push(value);
    let mut value: MaintenanceAuditEvent = serde_json::from_value(old_json()).unwrap();
    value.target.target_revision = None;
    invalid.push(value);
    for value in invalid {
        let ledger = MemoryAuditLedger::new();
        assert!(audit::append(&ledger, &value).is_err());
        assert!(ledger.query(&AuditFilter::default()).unwrap().is_empty());
    }
}
