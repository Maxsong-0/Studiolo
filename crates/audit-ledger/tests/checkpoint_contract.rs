//! 真实账本封存与跨库 ACK 证明，不接受同 offset 的其他事件或损坏链。
#![cfg(unix)]
mod support;
use serde_json::json;
use std::time::{Duration, Instant};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, ReceiptCoverage,
    SanitizedPayload, SqliteAuditLedger,
};
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn event(value: &str) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse("checkpoint-test").unwrap(),
        AuditPhase::Outcome,
        "checkpoint.test",
        SanitizedPayload::try_from(json!({"decision": value})).unwrap(),
    )
}
#[test]
fn frozen_ledger_checks_original_event_and_exports_an_independent_cutoff() {
    let source = support::private_dir();
    let other = support::private_dir();
    let target = support::private_dir();
    let ledger = SqliteAuditLedger::open(source.path()).unwrap();
    let record = ledger.append(event("allow")).unwrap();
    let wrong = SqliteAuditLedger::open(other.path())
        .unwrap()
        .append(event("deny"))
        .unwrap();
    assert_eq!(record.offset(), wrong.offset());
    let handle = ledger.checkpoint_source().unwrap();
    let quiescence = ledger.quiesce_operations(deadline()).unwrap();
    let frozen = handle.freeze(deadline()).unwrap();
    let mut coverage = ReceiptCoverage::default();
    coverage
        .include(frozen.verify_receipt(&record).unwrap())
        .unwrap();
    coverage
        .include(frozen.verify_event(record.event()).unwrap())
        .unwrap();
    assert_eq!(coverage.checked(), 2);
    assert_eq!(coverage.maximum_offset(), 1);
    assert!(frozen.verify_receipt(&wrong).is_err());
    assert!(frozen.verify_event(wrong.event()).is_err());
    let sealed = frozen.export(target.path()).unwrap();
    assert_eq!(&sealed.watermark, frozen.watermark());
    let copy = SqliteAuditLedger::open_existing(target.path()).unwrap();
    assert_eq!(copy.query(&AuditFilter::default()).unwrap(), vec![record]);
    let writer = rusqlite::Connection::open(source.path().join("audit.sqlite3")).unwrap();
    writer.busy_timeout(Duration::ZERO).unwrap();
    assert!(writer.execute_batch("BEGIN IMMEDIATE").is_err());
    drop(frozen);
    drop(quiescence);
    writer.execute_batch("BEGIN IMMEDIATE;ROLLBACK").unwrap();
}
#[test]
fn source_capability_rejects_replacement_and_freeze_verifies_the_entire_chain() {
    let source = support::private_dir();
    let ledger = SqliteAuditLedger::open(source.path()).unwrap();
    ledger.append(event("allow")).unwrap();
    let handle = ledger.checkpoint_source().unwrap();
    let writer = rusqlite::Connection::open(source.path().join("audit.sqlite3")).unwrap();
    writer
        .execute(
            "UPDATE audit_events SET record_json=replace(record_json,'allow','deny')",
            [],
        )
        .unwrap();
    assert!(handle.freeze(deadline()).is_err());
    drop(writer);
    std::fs::rename(
        source.path().join("audit.sqlite3"),
        source.path().join("replaced.sqlite3"),
    )
    .unwrap();
    std::fs::copy(
        source.path().join("replaced.sqlite3"),
        source.path().join("audit.sqlite3"),
    )
    .unwrap();
    assert!(handle.freeze(deadline()).is_err());
}
