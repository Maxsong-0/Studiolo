use super::{super::audit, rename_fixture::event};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord, MemoryAuditLedger,
    SanitizedPayload,
};
use studiolo_vault_broker::{FileOperationPhase, MaintenanceAuditEvent};

#[test]
fn old_typed_schema_two_has_no_rename_fields_and_keeps_exact_payload_bytes() {
    let old = serde_json::json!({
        "schema_version":2,"event_id":"1".repeat(64),"phase":"intent",
        "vault_uuid":"57c07e80-49f2-4493-9268-a409d473701a",
        "actor":{"kind":"local_os","uid":501,"gid":20},
        "decision":"cancel_prepared","requested_at_ms":null,
        "target":{
            "review_id":"2".repeat(64),"request_fingerprint":"3".repeat(64),
            "path":"note.md","kind":"prepared","sequence":1,
            "previous_revision":null,"target_revision":null,"file_operation_id":"4".repeat(64),
            "current":{"kind":"not_observed"},"proposed_available":true
        },"outcome":null,"resolution":null,"evidence_id":null
    });
    let value: MaintenanceAuditEvent = serde_json::from_value(old.clone()).unwrap();
    assert!(value.target.secondary.is_none() && value.target.file_operation.is_none());
    assert_eq!(
        serde_json::to_vec(&serde_json::to_value(&value).unwrap()).unwrap(),
        serde_json::to_vec(&old).unwrap()
    );
    let ledger = MemoryAuditLedger::new();
    audit::append(&ledger, &value).unwrap();
    let mut expected = old;
    expected.as_object_mut().unwrap().remove("event_id");
    expected.as_object_mut().unwrap().remove("phase");
    let rows = ledger.query(&AuditFilter::default()).unwrap();
    assert_eq!(
        serde_json::to_vec(rows[0].event().payload().as_value()).unwrap(),
        serde_json::to_vec(&expected).unwrap()
    );
}

struct WrongTarget(MemoryAuditLedger);
impl AuditLedger for WrongTarget {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let mut payload = event.payload().as_value().clone();
        payload["target"]["secondary"]["path"] = "other.md".into();
        self.0.append(AuditEvent::new(
            event.event_id().clone(),
            event.phase(),
            event.action(),
            SanitizedPayload::try_from(payload).unwrap(),
        ))
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.0.query(filter)
    }
}

#[test]
fn valid_record_hash_for_changed_secondary_target_is_never_an_ack() {
    let ledger = WrongTarget(MemoryAuditLedger::new());
    assert!(audit::append(&ledger, &event(FileOperationPhase::Prepared)).is_err());
    let rows = ledger.query(&AuditFilter::default()).unwrap();
    assert_eq!(rows.len(), 1);
    rows[0].verify_integrity().unwrap();
}
