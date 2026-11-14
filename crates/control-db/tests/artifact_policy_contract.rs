//! 真实 SQLite 批准事务、原审计、不可变重放及旧库迁移的契约。
use rusqlite::{Connection, params};
use serde_json::json;
use studiolo_control_db::{AttemptRecord, ControlDb, SnapshotRecord};
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, Fence, ModelEgress,
    SourcePolicyRef, TaskId, VaultUuid,
};
fn fact(expiry: i64) -> ArtifactProvenance {
    ArtifactProvenance::new(
        SourcePolicyRef::new("fixture-project", "a".repeat(64)).unwrap(),
        DataLabels::new(
            Classification::Restricted,
            BackupScope::Full,
            DesktopSync::Deny,
            ModelEgress::Deny,
        )
        .unwrap(),
        expiry,
        "b".repeat(64),
    )
    .unwrap()
}
fn claim(db: &mut ControlDb, vault: VaultUuid) -> AttemptRecord {
    let task = TaskId::new();
    db.create_task(task, &json!({"title":"fixture"}), 1)
        .unwrap();
    db.queue_task(task, 2).unwrap();
    db.claim(
        task,
        "fixture",
        SnapshotRecord {
            snapshot_id: task.to_string(),
            vault_uuid: vault,
            entries: vec![],
        },
        3,
        100_000,
    )
    .unwrap()
}
fn approve(
    db: &mut ControlDb,
    a: &AttemptRecord,
    expiry: i64,
    now: i64,
) -> Result<(), studiolo_control_db::DbError> {
    db.authorize_artifacts(
        a.task_id,
        a.attempt_id,
        a.fence,
        &a.snapshot_id,
        &fact(expiry),
        now,
    )
}
#[test]
fn original_authorization_survives_reopen_exact_replay_and_expiry_without_new_audit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let mut db = ControlDb::open(&path, vault).unwrap();
    let a = claim(&mut db, vault);
    assert!(
        db.artifact_provenance(a.task_id, a.attempt_id, a.fence)
            .unwrap()
            .is_none()
    );
    approve(&mut db, &a, 10, 4).unwrap();
    let before = db.pending_runtime_audits().unwrap();
    let event = before
        .iter()
        .find(|e| e.action == "task.artifacts.authorized")
        .unwrap();
    assert_eq!(event.payload["approved_at_ms"], 4);
    assert_eq!(event.payload["snapshot_id"], a.snapshot_id);
    db.attach_runner(
        a.task_id,
        a.attempt_id,
        a.fence,
        &json!({"opaque":"fixture"}),
        5,
    )
    .unwrap();
    drop(db);
    let mut db = ControlDb::open(&path, vault).unwrap();
    approve(&mut db, &a, 10, 200_000).unwrap();
    assert_eq!(db.pending_runtime_audits().unwrap(), before);
    let original = db
        .artifact_provenance(a.task_id, a.attempt_id, a.fence)
        .unwrap()
        .unwrap();
    assert_eq!(original, fact(10));
    assert!(!original.is_live_at(200_000));
    assert!(approve(&mut db, &a, 20, 6).is_err());
    assert!(
        db.authorize_artifacts(a.task_id, a.attempt_id, a.fence, "other", &fact(10), 6)
            .is_err()
    );
    assert!(
        db.artifact_provenance(
            a.task_id,
            a.attempt_id,
            Fence::new(a.fence.get() + 1).unwrap()
        )
        .is_err()
    );
    assert_eq!(db.pending_runtime_audits().unwrap(), before);
}
#[test]
fn first_approval_is_rejected_after_launch_cancel_expiry_or_before_claim_time() {
    for kind in ["launch", "cancel", "expired", "old-clock", "lease"] {
        let vault = VaultUuid::new();
        let mut db = ControlDb::open_in_memory(vault).unwrap();
        let a = claim(&mut db, vault);
        if kind == "launch" {
            db.attach_runner(a.task_id, a.attempt_id, a.fence, &json!({}), 4)
                .unwrap();
        }
        if kind == "cancel" {
            db.cancel_task(a.task_id, 4).unwrap();
        }
        let before = db.pending_runtime_audits().unwrap();
        let now = match kind {
            "expired" => 10_000,
            "old-clock" => 2,
            "lease" => 100_000,
            _ => 5,
        };
        assert!(approve(&mut db, &a, 10, now).is_err(), "{kind}");
        assert!(
            db.artifact_provenance(a.task_id, a.attempt_id, a.fence)
                .unwrap()
                .is_none()
        );
        assert_eq!(db.pending_runtime_audits().unwrap(), before);
    }
}
#[test]
fn conflicting_audit_identity_rolls_back_entire_approval() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let mut db = ControlDb::open(&path, vault).unwrap();
    let a = claim(&mut db, vault);
    let raw = Connection::open(&path).unwrap();
    raw.execute(
        "INSERT INTO runtime_audit_outbox(event_id,action,payload_json) VALUES(?1,'wrong','{}')",
        [format!("artifact-policy:{}", a.attempt_id)],
    )
    .unwrap();
    let before = db.pending_runtime_audits().unwrap();
    assert!(approve(&mut db, &a, 10, 4).is_err());
    assert!(
        db.artifact_provenance(a.task_id, a.attempt_id, a.fence)
            .unwrap()
            .is_none()
    );
    assert_eq!(db.pending_runtime_audits().unwrap(), before);
}
#[test]
fn sqlite_enforces_immutable_policy_and_original_attempt_binding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let mut db = ControlDb::open(&path, vault).unwrap();
    let a = claim(&mut db, vault);
    let b = claim(&mut db, vault);
    approve(&mut db, &a, 10, 4).unwrap();
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    assert!(raw.execute("DELETE FROM artifact_policies", []).is_err());
    assert!(
        raw.execute("UPDATE artifact_policies SET approved_at_ms=5", [])
            .is_err()
    );
    assert!(
        raw.execute(
            "UPDATE attempts SET snapshot_id=?1 WHERE attempt_id=?2",
            params![b.snapshot_id, a.attempt_id.to_string()]
        )
        .is_err()
    );
    raw.execute("UPDATE runtime_audit_outbox SET payload_json='{}' WHERE action='task.artifacts.authorized'", []).unwrap();
    assert!(
        db.artifact_provenance(a.task_id, a.attempt_id, a.fence)
            .is_err()
    );
}
#[test]
fn v3_upgrade_keeps_legacy_attempt_without_an_invented_full_policy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let mut db = ControlDb::open(&path, vault).unwrap();
    let a = claim(&mut db, vault);
    drop(db);
    let raw = Connection::open(&path).unwrap();
    raw.execute_batch("DROP TABLE artifact_policies; DROP INDEX artifact_attempt_identity; DELETE FROM schema_migrations WHERE version=4;").unwrap();
    drop(raw);
    let db = ControlDb::open(&path, vault).unwrap();
    assert_eq!(db.schema_version().unwrap(), 4);
    assert!(
        db.artifact_provenance(a.task_id, a.attempt_id, a.fence)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.attempt(a.task_id, a.attempt_id)
            .unwrap()
            .unwrap()
            .snapshot_id,
        a.snapshot_id
    );
    assert!(
        db.pending_runtime_audits()
            .unwrap()
            .iter()
            .all(|e| e.action != "task.artifacts.authorized")
    );
}
