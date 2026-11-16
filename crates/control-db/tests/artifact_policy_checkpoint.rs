//! 原批准必须在实际审计账本中确认后才可封存；损坏元数据不能被导出。
#![cfg(unix)]
use serde_json::json;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditLedger, SqliteAuditLedger};
use studiolo_control_db::{ControlDb, SnapshotRecord};
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef, TaskId, VaultUuid,
};
fn directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
#[test]
fn actual_checkpoint_requires_exact_original_policy_audit_and_rejects_corruption() {
    for damage in [false, true] {
        let owner = directory();
        let audit = directory();
        let copy = directory();
        let vault = VaultUuid::new();
        let path = owner.path().join("control.sqlite3");
        fs::File::create(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let mut db = ControlDb::open_checkpoint_ready(owner.path(), vault).unwrap();
        let source = db.checkpoint_source().unwrap();
        let task = TaskId::new();
        db.create_task(task, &json!({"title":"fixture"}), 1)
            .unwrap();
        db.queue_task(task, 2).unwrap();
        let a = db
            .claim(
                task,
                "fixture",
                SnapshotRecord {
                    snapshot_id: "original".into(),
                    vault_uuid: vault,
                    entries: vec![],
                },
                3,
                100,
            )
            .unwrap();
        let provenance = ArtifactProvenance::new(
            SourcePolicyRef::new("fixture", "a".repeat(64)).unwrap(),
            DataLabels::new(
                Classification::Restricted,
                BackupScope::Full,
                DesktopSync::Deny,
                ModelEgress::Deny,
            )
            .unwrap(),
            10,
            "b".repeat(64),
        )
        .unwrap();
        db.authorize_artifacts(task, a.attempt_id, a.fence, &a.snapshot_id, &provenance, 4)
            .unwrap();
        for record in db.pending_outbox().unwrap() {
            db.mark_outbox_delivered(record.sequence, 5).unwrap();
        }
        assert!(source.freeze(deadline()).is_err());
        let ledger = SqliteAuditLedger::open(audit.path()).unwrap();
        for event in db.pending_runtime_audits().unwrap() {
            ledger.append(event.audit_event().unwrap()).unwrap();
            db.acknowledge_runtime_audit(&event.event_id).unwrap();
        }
        if damage {
            let raw = rusqlite::Connection::open(&path).unwrap();
            raw.execute("UPDATE runtime_audit_outbox SET payload_json='{}' WHERE action='task.artifacts.authorized'", []).unwrap();
            assert!(source.freeze(deadline()).is_err());
        } else {
            let mut frozen = source.freeze(deadline()).unwrap();
            let audit = ledger
                .checkpoint_source()
                .unwrap()
                .freeze(deadline())
                .unwrap();
            assert!(frozen.verify_audit(&audit).unwrap().checked() >= 2);
            frozen.export(copy.path()).unwrap();
            let restored = ControlDb::open_checkpoint_ready(copy.path(), vault).unwrap();
            assert_eq!(
                restored
                    .artifact_provenance(task, a.attempt_id, a.fence)
                    .unwrap(),
                Some(provenance)
            );
        }
    }
}
