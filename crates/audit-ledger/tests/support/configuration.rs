#![cfg(unix)]

use super::support_tests as support;

use super::SqliteAuditLedger;
use crate::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, SanitizedPayload,
};

fn event(id: &str, summary: &str) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(id).unwrap(),
        AuditPhase::Intent,
        "test.capacity",
        SanitizedPayload::try_from(serde_json::json!({"summary": summary})).unwrap(),
    )
}

#[test]
fn owned_connection_uses_the_required_durability_configuration() {
    let directory = support::private_dir();
    let ledger = SqliteAuditLedger::open(&directory.path().canonicalize().unwrap()).unwrap();
    // 只检查 Adapter 私有实现，不为断言向生产调用者暴露连接或写 SQL 的入口。
    let session = ledger.connection.lock().unwrap();
    let connection = &session.connection;
    let mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    for (pragma, expected) in [
        ("PRAGMA synchronous", 2),
        ("PRAGMA fullfsync", 1),
        ("PRAGMA checkpoint_fullfsync", 1),
        ("PRAGMA foreign_keys", 1),
        ("PRAGMA trusted_schema", 0),
        ("PRAGMA busy_timeout", 5000),
    ] {
        assert_eq!(
            connection
                .query_row(pragma, [], |row| row.get::<_, i64>(0))
                .unwrap(),
            expected
        );
    }
}

#[test]
fn sqlite_full_rolls_back_head_and_retry_appends_only_once() {
    let directory = support::private_dir();
    let root = directory.path().canonicalize().unwrap();
    let ledger = SqliteAuditLedger::open(&root).unwrap();
    let first = ledger.append(event("before-full", "small")).unwrap();
    let summary = "x".repeat(256 * 1024);
    {
        let session = ledger.connection.lock().unwrap();
        let connection = &session.connection;
        let pages: i64 = connection
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        let maximum: i64 = connection
            .query_row(&format!("PRAGMA max_page_count={pages}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(maximum, pages);
        // 先证明故障是真实 SQLITE_FULL，不把忙锁或内存替身冒充存储容量失败。
        let failure = connection.execute(
            "INSERT INTO audit_events(offset,event_id,phase,record_json) VALUES(2,'full-probe','intent',?1)",
            [&summary]).unwrap_err();
        assert!(matches!(failure, rusqlite::Error::SqliteFailure(error, _)
            if error.code == rusqlite::ErrorCode::DiskFull));
    }
    let retry = event("during-full", &summary);
    assert_eq!(
        ledger.append(retry.clone()).unwrap_err(),
        AuditError::Unavailable
    );
    assert_eq!(
        ledger.query(&AuditFilter::default()).unwrap(),
        vec![first.clone()]
    );
    {
        let session = ledger.connection.lock().unwrap();
        let connection = &session.connection;
        let maximum: i64 = connection
            .query_row("PRAGMA max_page_count=2147483646", [], |row| row.get(0))
            .unwrap();
        assert_eq!(maximum, 2147483646);
    }
    let second = ledger.append(retry.clone()).unwrap();
    assert_eq!(second.offset(), 2);
    assert_eq!(second.previous_hash(), first.record_hash());
    assert_eq!(ledger.append(retry).unwrap(), second);
    drop(ledger);
    let reopened = SqliteAuditLedger::open(&root).unwrap();
    assert_eq!(
        reopened.query(&AuditFilter::default()).unwrap(),
        vec![first, second]
    );
}
