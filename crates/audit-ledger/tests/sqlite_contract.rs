//! 持久审计的重启、过滤与并发幂等契约。

#![cfg(unix)]

mod support;

use serde_json::json;
use std::{
    os::unix::fs::PermissionsExt,
    sync::{Arc, Barrier},
    thread,
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditFilter, AuditLedger, AuditPhase, AuditSubject,
    IntegrityHash, SanitizedPayload, SqliteAuditLedger,
};
use studiolo_core::{AttemptId, TaskId, ToolId};

fn event(id: &str, phase: AuditPhase) -> AuditEvent {
    AuditEvent::new(
        AuditEventId::parse(id).unwrap(),
        phase,
        "session.establish",
        SanitizedPayload::try_from(json!({"decision": "allow"})).unwrap(),
    )
}

#[test]
fn reopen_preserves_exact_records_and_idempotency() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    let first = SqliteAuditLedger::open(&root).unwrap();
    let original = event("login-1", AuditPhase::Intent);
    let record = first.append(original.clone()).unwrap();
    assert_eq!(record.previous_hash(), IntegrityHash::genesis());
    drop(first);
    let reopened = SqliteAuditLedger::open(&root).unwrap();
    assert_eq!(reopened.append(original).unwrap(), record);
    let outcome = reopened
        .append(event("login-1", AuditPhase::Outcome))
        .unwrap();
    assert_eq!(outcome.offset(), 2);
    assert_eq!(outcome.previous_hash(), record.record_hash());
    let conflict = AuditEvent::new(
        AuditEventId::parse("login-1").unwrap(),
        AuditPhase::Intent,
        "different",
        SanitizedPayload::try_from(json!({})).unwrap(),
    );
    assert!(matches!(
        reopened.append(conflict),
        Err(AuditError::IdempotencyConflict { .. })
    ));
    assert_eq!(
        reopened.query(&AuditFilter::default()).unwrap(),
        vec![record, outcome]
    );
    assert_eq!(
        std::fs::metadata(root.join("audit.sqlite3"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let connection = rusqlite::Connection::open(root.join("audit.sqlite3")).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA journal_mode", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
}

#[test]
fn durable_filters_match_the_public_seam() {
    let dir = support::private_dir();
    let ledger = SqliteAuditLedger::open(&dir.path().canonicalize().unwrap()).unwrap();
    let task = TaskId::new();
    let attempt = AttemptId::new();
    let subject = AuditSubject::default()
        .with_task(task)
        .with_attempt(attempt)
        .with_tool(ToolId::from_static("web.fetch"));
    let first = ledger
        .append(event("match", AuditPhase::Intent).with_subject(subject))
        .unwrap();
    ledger.append(event("match", AuditPhase::Outcome)).unwrap();
    let filter = AuditFilter::default()
        .with_event_id(AuditEventId::parse("match").unwrap())
        .with_task(task)
        .with_attempt(attempt)
        .with_tool(ToolId::from_static("web.fetch"))
        .with_phase(AuditPhase::Intent)
        .with_action("session.establish")
        .after_offset(0)
        .with_limit(1);
    assert_eq!(ledger.query(&filter).unwrap(), vec![first]);
    assert!(
        ledger
            .query(&AuditFilter::default().after_offset(2))
            .unwrap()
            .is_empty()
    );
    assert!(
        ledger
            .query(&AuditFilter::default().with_limit(0))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn independent_instances_serialize_and_same_key_only_appends_once() {
    let dir = support::private_dir();
    let root = dir.path().canonicalize().unwrap();
    drop(SqliteAuditLedger::open(&root).unwrap());
    let ledgers: Vec<_> = (0..4)
        .map(|_| SqliteAuditLedger::open(&root).unwrap())
        .collect();
    let barrier = Arc::new(Barrier::new(4));
    let handles: Vec<_> = ledgers
        .into_iter()
        .enumerate()
        .map(|(worker, ledger)| {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                let shared = ledger.append(event("same", AuditPhase::Intent)).unwrap();
                for index in 0..8 {
                    ledger
                        .append(event(&format!("{worker}-{index}"), AuditPhase::Intent))
                        .unwrap();
                }
                shared
            })
        })
        .collect();
    let records: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert!(records.windows(2).all(|pair| pair[0] == pair[1]));
    let reopened = SqliteAuditLedger::open(&root).unwrap();
    let all = reopened.query(&AuditFilter::default()).unwrap();
    assert_eq!(all.len(), 33);
    for (index, record) in all.iter().enumerate() {
        assert_eq!(record.offset(), index as u64 + 1);
        if index > 0 {
            assert_eq!(record.previous_hash(), all[index - 1].record_hash());
        }
    }
}
