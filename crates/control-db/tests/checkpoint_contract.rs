//! 真实 Control/Audit 以及三类 outbox；导出不能越过待投递状态。
#![cfg(unix)]
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};
use studiolo_control_db::{
    ControlDb, RuntimeAuditRecord, SnapshotRecord, TaskStatus, TerminalReservation,
};
use studiolo_core::{Revision, TaskId, VaultPath, VaultUuid};
fn directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
fn database(dir: &tempfile::TempDir, vault: VaultUuid) -> ControlDb {
    let path = dir.path().join("control.sqlite3");
    fs::File::create(&path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    ControlDb::open_checkpoint_ready(dir.path(), vault).unwrap()
}
fn drain_projection(db: &mut ControlDb) {
    for row in db.pending_outbox().unwrap() {
        db.mark_outbox_delivered(row.sequence, 100).unwrap();
    }
}
#[test]
fn actual_private_owner_is_pinned_before_initialization_and_export_is_independent() {
    let dir = directory();
    let audit = directory();
    let target = directory();
    let vault = VaultUuid::new();
    let db = database(&dir, vault);
    let ledger = SqliteAuditLedger::open(audit.path()).unwrap();
    assert!(
        ControlDb::open_in_memory(vault)
            .unwrap()
            .checkpoint_source()
            .is_err()
    );
    let source = db.checkpoint_source().unwrap();
    let mut frozen = source.freeze(deadline()).unwrap();
    frozen.validate().unwrap();
    assert!(frozen.export(target.path()).is_err());
    let audit = ledger
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    assert_eq!(frozen.verify_audit(&audit).unwrap().checked(), 0);
    frozen.export(target.path()).unwrap();
    let copy = ControlDb::open_checkpoint_ready(target.path(), vault).unwrap();
    assert_eq!(copy.bound_vault_uuid().unwrap(), vault);
    let raw = rusqlite::Connection::open(dir.path().join("control.sqlite3")).unwrap();
    raw.busy_timeout(Duration::ZERO).unwrap();
    assert!(raw.execute_batch("BEGIN IMMEDIATE").is_err());
    drop(frozen);
    raw.execute_batch("BEGIN IMMEDIATE;ROLLBACK").unwrap();
    fs::rename(
        dir.path().join("control.sqlite3"),
        dir.path().join("old.sqlite3"),
    )
    .unwrap();
    fs::copy(
        dir.path().join("old.sqlite3"),
        dir.path().join("control.sqlite3"),
    )
    .unwrap();
    assert!(source.freeze(deadline()).is_err());
}
#[test]
fn projections_and_runtime_audits_must_be_drained_and_acknowledged_in_original_ledger() {
    let dir = directory();
    let audit = directory();
    let empty = directory();
    let target = directory();
    let vault = VaultUuid::new();
    let mut db = database(&dir, vault);
    let source = db.checkpoint_source().unwrap();
    db.create_task(TaskId::new(), &json!({"title":"test"}), 1)
        .unwrap();
    assert!(source.freeze(deadline()).unwrap().validate().is_err());
    drain_projection(&mut db);
    let raw = rusqlite::Connection::open(dir.path().join("control.sqlite3")).unwrap();
    raw.execute("INSERT INTO runtime_audit_outbox(event_id,action,payload_json) VALUES('checkpoint-runtime','task.test','{}')",[]).unwrap();
    assert!(source.freeze(deadline()).unwrap().validate().is_err());
    let ledger = SqliteAuditLedger::open(audit.path()).unwrap();
    let record = RuntimeAuditRecord {
        event_id: "checkpoint-runtime".into(),
        action: "task.test".into(),
        payload: json!({}),
    };
    ledger.append(record.audit_event().unwrap()).unwrap();
    db.acknowledge_runtime_audit(&record.event_id).unwrap();
    let mut frozen = source.freeze(deadline()).unwrap();
    frozen.validate().unwrap();
    let wrong = SqliteAuditLedger::open(empty.path())
        .unwrap()
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    assert!(frozen.verify_audit(&wrong).is_err());
    let audit = ledger
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    assert_eq!(frozen.verify_audit(&audit).unwrap().checked(), 1);
    frozen.export(target.path()).unwrap();
}
#[test]
fn terminal_commit_requires_durable_outcome_and_checks_complete_event() {
    let dir = directory();
    let audit = directory();
    let target = directory();
    let vault = VaultUuid::new();
    let mut db = database(&dir, vault);
    let source = db.checkpoint_source().unwrap();
    let task = TaskId::new();
    db.create_task(task, &json!({"title":"test"}), 1).unwrap();
    db.queue_task(task, 2).unwrap();
    let attempt = db
        .claim(
            task,
            "checkpoint-worker",
            SnapshotRecord {
                snapshot_id: "fixed".into(),
                vault_uuid: vault,
                entries: vec![],
            },
            3,
            103,
        )
        .unwrap();
    let reservation = TerminalReservation {
        report_json: json!({"format":3,"event_id":"checkpoint-terminal","task_id":task,"attempt_id":attempt.attempt_id,"fence":attempt.fence}),
        artifacts_json: json!({"id":"a".repeat(64),"sha256":"b".repeat(64)}),
    };
    db.reserve_terminal(task, attempt.attempt_id, attempt.fence, &reservation, 4)
        .unwrap();
    drain_projection(&mut db);
    assert!(source.freeze(deadline()).unwrap().validate().is_err());
    let path = VaultPath::parse("artifacts/checkpoint.json").unwrap();
    let receipt = json!({"manifest_path":path,"manifest_revision":Revision::from_bytes([7;32]),"artifacts":[]});
    db.finalize_terminal(
        task,
        attempt.attempt_id,
        attempt.fence,
        TaskStatus::NeedsReview,
        Some(&receipt),
        5,
    )
    .unwrap();
    drain_projection(&mut db);
    assert!(source.freeze(deadline()).unwrap().validate().is_err());
    let ledger = SqliteAuditLedger::open(audit.path()).unwrap();
    for outcome in db.pending_terminal_audits().unwrap() {
        ledger.append(outcome.audit_event().unwrap()).unwrap();
        db.mark_terminal_audit_delivered(outcome.attempt.attempt_id, 6)
            .unwrap();
    }
    for event in db.pending_runtime_audits().unwrap() {
        ledger.append(event.audit_event().unwrap()).unwrap();
        db.acknowledge_runtime_audit(&event.event_id).unwrap();
    }
    let mut frozen = source.freeze(deadline()).unwrap();
    frozen.validate().unwrap();
    let audit = ledger
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    assert!(frozen.verify_audit(&audit).unwrap().checked() >= 1);
    frozen.export(target.path()).unwrap();
}
