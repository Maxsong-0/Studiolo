//! 损坏审计正文、行键与元数据的读取字节预算。

#![cfg(unix)]

mod support;

use serde_json::json;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, SanitizedPayload,
    SqliteAuditLedger,
};

fn event() -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse("budget-event").unwrap(),
        AuditPhase::Intent,
        "test.read_budget",
        SanitizedPayload::try_from(json!({"decision": "allow"})).unwrap(),
    )
}

fn reject_changed_field(update: &str, value: &str, expected: AuditError) {
    let directory = support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    ledger.append(event()).unwrap();
    let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    // 仅测试连接允许构造非法 phase；被测连接仍须完整验链与检查约束。
    connection
        .execute_batch("PRAGMA ignore_check_constraints=ON")
        .unwrap();
    connection.execute(update, [value]).unwrap();
    drop(connection);

    assert_eq!(
        ledger.query(&AuditFilter::default().with_limit(0)),
        Err(expected.clone()),
    );
    assert_eq!(ledger.append(event()), Err(expected.clone()));
    drop(ledger);
    assert_eq!(SqliteAuditLedger::open(&root).err(), Some(expected));
}

#[test]
fn oversized_record_and_independent_row_keys_are_rejected() {
    // 字符数未超预算，UTF-8 字节数超过 1 MiB，不能误用 length(text) 作限制。
    let oversized = "中".repeat(400_000);
    assert!(oversized.chars().count() < 1024 * 1024);
    assert!(oversized.len() > 1024 * 1024);
    for update in [
        "UPDATE audit_events SET record_json=?1",
        "UPDATE audit_events SET event_id=?1",
        "UPDATE audit_events SET phase=?1",
    ] {
        reject_changed_field(update, &oversized, AuditError::CorruptStore);
    }
}

#[test]
fn metadata_text_is_restricted_to_its_fixed_byte_lengths() {
    let oversized = "x".repeat(1024 * 1024 + 1);
    reject_changed_field(
        "UPDATE audit_meta SET codec=?1",
        &oversized,
        AuditError::UnsupportedSchema,
    );
    reject_changed_field(
        "UPDATE audit_meta SET last_hash=?1",
        &oversized,
        AuditError::CorruptStore,
    );
    // 看似 64 字符的文本不是 64 字节 SHA-256 hex。
    reject_changed_field(
        "UPDATE audit_meta SET last_hash=?1",
        &"中".repeat(64),
        AuditError::CorruptStore,
    );
}

#[test]
fn oversized_schema_declaration_is_rejected_with_only_the_four_expected_objects() {
    let directory = support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    ledger.append(event()).unwrap();
    drop(ledger);
    let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    // 追加空白保留 SQLite 可解析的原表；只让其声明超过应用的读取预算。
    connection
        .execute_batch("PRAGMA writable_schema=ON")
        .unwrap();
    connection
        .execute(
            "UPDATE sqlite_schema SET sql=sql || ?1 WHERE name='audit_events'",
            [" ".repeat(4096)],
        )
        .unwrap();
    connection
        .execute_batch("PRAGMA writable_schema=OFF")
        .unwrap();
    let count: i64 = connection
        .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
        .unwrap();
    let bytes: i64 = connection
        .query_row(
            "SELECT octet_length(sql) FROM sqlite_schema WHERE name='audit_events'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // v2 精确 schema：audit_meta、audit_events、audit_segments 与唯一索引，共四个对象。
    assert_eq!(count, 4);
    assert!(bytes > 1024);
    drop(connection);
    assert_eq!(
        SqliteAuditLedger::open(&root).err(),
        Some(AuditError::CorruptStore)
    );
}

#[test]
fn extra_schema_objects_are_rejected_before_collecting_declarations() {
    let directory = support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    connection
        .execute_batch("CREATE TABLE unexpected(value TEXT)")
        .unwrap();
    drop(connection);
    assert_eq!(
        ledger.query(&AuditFilter::default()),
        Err(AuditError::CorruptStore)
    );
    drop(ledger);
    assert_eq!(
        SqliteAuditLedger::open(&root).err(),
        Some(AuditError::CorruptStore)
    );
}

#[test]
fn bounded_chinese_keys_and_payloads_still_roundtrip() {
    let directory = support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    let original = AuditEvent::new(
        AuditEventId::parse("文".repeat(40_000)).unwrap(),
        AuditPhase::Intent,
        "test.read_budget",
        SanitizedPayload::try_from(json!({"summary": "中".repeat(10_000)})).unwrap(),
    );
    let record = ledger.append(original.clone()).unwrap();
    drop(ledger);
    let reopened = SqliteAuditLedger::open(&root).unwrap();
    assert_eq!(reopened.append(original).unwrap(), record);
    assert_eq!(
        reopened.query(&AuditFilter::default()).unwrap(),
        vec![record]
    );
}
