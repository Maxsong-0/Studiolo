//! 持久审计的损坏、未知格式与忙锁失败契约。

#![cfg(unix)]

mod support;

use serde_json::json;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, SanitizedPayload,
    SqliteAuditLedger,
};

fn event() -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse("test").unwrap(),
        AuditPhase::Intent,
        "test.action",
        SanitizedPayload::try_from(json!({"decision": "allow"})).unwrap(),
    )
}

#[test]
fn restart_rejects_schema_head_offset_and_record_tampering() {
    for change in [
        "PRAGMA user_version=99",
        "PRAGMA application_id=0",
        "UPDATE audit_meta SET codec='unknown'",
        "UPDATE audit_meta SET schema_version=2",
        "UPDATE audit_meta SET last_offset=0",
        "UPDATE audit_meta SET last_hash='invalid'",
        "DELETE FROM audit_meta",
        "DELETE FROM audit_events",
        "UPDATE audit_events SET offset=2",
        "UPDATE audit_events SET event_id='changed'",
        "UPDATE audit_events SET phase='outcome'",
        "UPDATE audit_events SET record_json=replace(record_json,'allow','deny')",
        "UPDATE audit_events SET record_json=record_json || ' '",
        "CREATE TABLE unexpected(value TEXT)",
        "CREATE TRIGGER unexpected AFTER INSERT ON audit_events BEGIN DELETE FROM audit_events; END",
    ] {
        let dir = support::private_dir();
        let root = dir.path().canonicalize().unwrap();
        let ledger = SqliteAuditLedger::open(&root).unwrap();
        ledger.append(event()).unwrap();
        drop(ledger);
        let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
        connection.execute_batch(change).unwrap();
        drop(connection);
        assert!(SqliteAuditLedger::open(&root).is_err(), "accepted {change}");
    }
}

#[test]
fn open_ledger_rechecks_before_query_or_append() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    ledger.append(event()).unwrap();
    let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    connection
        .execute("UPDATE audit_meta SET last_hash='changed'", [])
        .unwrap();
    assert!(ledger.query(&AuditFilter::default().with_limit(0)).is_err());
    assert!(ledger.append(event()).is_err());
}

#[test]
fn sqlite_writer_lock_fails_closed_and_retry_is_exactly_once() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert_eq!(ledger.append(event()).unwrap_err(), AuditError::Unavailable);
    connection.execute_batch("ROLLBACK").unwrap();
    assert_eq!(ledger.append(event()).unwrap().offset(), 1);
    assert_eq!(ledger.append(event()).unwrap().offset(), 1);
}

#[test]
fn existing_empty_or_non_sqlite_file_is_not_reinitialized() {
    use std::os::unix::fs::PermissionsExt;
    for contents in [b"".as_slice(), b"not a database".as_slice()] {
        let dir = support::private_dir();
        let root = dir.path().canonicalize().unwrap();
        let path = root.join("audit.sqlite3");
        std::fs::write(&path, contents).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(SqliteAuditLedger::open(&root).is_err());
    }
}
