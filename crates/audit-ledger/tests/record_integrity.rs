//! 对外回执自洽检查复用固定审计编码，不让 Auth 复制哈希公式。

use serde_json::json;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditRecord, IntegrityHash,
    MemoryAuditLedger, SanitizedPayload,
};

fn event(phase: AuditPhase) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse("fictional-receipt").unwrap(),
        phase,
        "session.issue",
        SanitizedPayload::try_from(json!({"decision":"allow","nested":{"b":2,"a":1}})).unwrap(),
    )
}

#[test]
fn authoritative_receipts_and_exact_replay_are_self_consistent() {
    let ledger = MemoryAuditLedger::new();
    let intent = event(AuditPhase::Intent);
    let first = ledger.append(intent.clone()).unwrap();
    let second = ledger.append(event(AuditPhase::Outcome)).unwrap();
    for record in [&first, &second, &ledger.append(intent).unwrap()] {
        record.verify_integrity().unwrap();
        let persisted = serde_json::to_string(record).unwrap();
        let restored: AuditRecord = serde_json::from_str(&persisted).unwrap();
        restored.verify_integrity().unwrap();
        assert_eq!(&restored, record);
    }
    assert_eq!(first.previous_hash(), IntegrityHash::genesis());
    assert_eq!(second.previous_hash(), first.record_hash());
}

#[test]
fn a_single_changed_receipt_field_is_rejected() {
    let ledger = MemoryAuditLedger::new();
    let record = ledger.append(event(AuditPhase::Intent)).unwrap();
    let original = serde_json::to_value(&record).unwrap();
    for (field, replacement) in [
        ("offset", json!(2)),
        ("recorded_at", json!("2024-01-01T00:00:00Z")),
        ("previous_hash", json!(vec![7_u8; 32])),
        ("record_hash", json!(vec![8_u8; 32])),
        (
            "event",
            serde_json::to_value(event(AuditPhase::Outcome)).unwrap(),
        ),
    ] {
        let mut changed = original.clone();
        changed[field] = replacement;
        let decoded: AuditRecord = serde_json::from_value(changed).unwrap();
        assert_eq!(
            decoded.verify_integrity(),
            Err(AuditError::CorruptStore),
            "{field}"
        );
    }
}

#[test]
fn zero_offset_and_non_genesis_first_predecessor_are_invalid() {
    let record = MemoryAuditLedger::new()
        .append(event(AuditPhase::Intent))
        .unwrap();
    for (offset, previous) in [
        (0, record.previous_hash()),
        (1, IntegrityHash::sha256(b"other")),
    ] {
        let invalid = AuditRecord::from_parts(
            offset,
            record.recorded_at(),
            previous,
            record.record_hash(),
            record.event().clone(),
        );
        assert_eq!(invalid.verify_integrity(), Err(AuditError::CorruptStore));
    }
}
