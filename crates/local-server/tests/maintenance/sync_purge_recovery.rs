//! 真正重开 PlaneState 与 SQLite 审计：模拟已落盘但回执丢失，以及删除中断。
use super::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
use studiolo_audit_ledger::{AuditError, AuditFilter, AuditRecord, SqliteAuditLedger};

struct Fixture {
    _root: tempfile::TempDir,
    plane: PathBuf,
    audit: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let plane = root.path().join("plane");
        let audit = root.path().join("audit");
        for path in [&plane, &audit] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        drop(SqliteAuditLedger::open(&audit).unwrap());
        Self {
            _root: root,
            plane,
            audit,
        }
    }
    fn store(&self) -> PlaneStateStore {
        PlaneStateStore::open(&self.plane).unwrap()
    }
    fn ledger(&self) -> SqliteAuditLedger {
        SqliteAuditLedger::open_existing(&self.audit).unwrap()
    }
    fn records(&self) -> Vec<AuditRecord> {
        self.ledger()
            .query(&AuditFilter::default().with_action(ACTION))
            .unwrap()
    }
}

/// 追加已经持久化，但指定阶段的第一次回执丢失；不会伪造内存账本。
struct LostAck {
    inner: SqliteAuditLedger,
    phase: AuditPhase,
    failed: AtomicBool,
}
impl AuditLedger for LostAck {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let phase = event.phase();
        let record = self.inner.append(event)?;
        if phase == self.phase && !self.failed.swap(true, Ordering::SeqCst) {
            Err(AuditError::Unavailable)
        } else {
            Ok(record)
        }
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}
fn planned() -> Counts {
    Counts {
        files: 3,
        directories: 2,
    }
}

#[test]
fn failed_root_revalidation_after_intent_closes_a_known_zero_outcome() {
    let f = Fixture::new();
    let mut state = f.store();
    let result = run(&f.ledger(), &mut state, "School", &planned(), |_| {
        Err(MaintenanceError::Configuration)
    });
    assert!(matches!(result, Err(MaintenanceError::Configuration)));
    assert!(state.state().pending_purge.is_none());
    let records = f.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].event().event_id(), records[1].event().event_id());
    assert_eq!(
        records[1].event().payload().as_value(),
        &json!({"schema_version":1,"folder":"School","files":0,"directories":0,"applied":false})
    );
}

#[test]
fn lost_intent_ack_reopens_without_deletion_and_reuses_original_event() {
    let f = Fixture::new();
    let ledger = LostAck {
        inner: f.ledger(),
        phase: AuditPhase::Intent,
        failed: AtomicBool::new(false),
    };
    let mut state = f.store();
    assert!(matches!(
        run(&ledger, &mut state, "School", &planned(), |_| panic!(
            "must not delete"
        )),
        Err(MaintenanceError::Audit)
    ));
    let id = state
        .state()
        .pending_purge
        .as_ref()
        .unwrap()
        .event_id
        .clone();
    drop(state);
    drop(ledger);
    let mut state = f.store();
    let report = recover(&f.ledger(), &mut state).unwrap().unwrap();
    assert_eq!(report["outcome_known"], true);
    assert_eq!(report["applied"], false);
    assert_eq!(report["removed_files"], 0);
    assert!(state.state().pending_purge.is_none());
    assert!(recover(&f.ledger(), &mut state).unwrap().is_none());
    let records = f.records();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|r| r.event().event_id().to_string() == id)
    );
}

#[test]
fn lost_outcome_ack_recovers_exact_counts_without_duplicate_audit_or_delete() {
    let f = Fixture::new();
    let marker = f._root.path().join("residue");
    fs::write(&marker, b"one").unwrap();
    let ledger = LostAck {
        inner: f.ledger(),
        phase: AuditPhase::Outcome,
        failed: AtomicBool::new(false),
    };
    let mut state = f.store();
    assert!(matches!(
        run(&ledger, &mut state, "School", &planned(), |removed| {
            fs::remove_file(&marker).unwrap();
            removed.files = 1;
            Ok(())
        }),
        Err(MaintenanceError::Audit)
    ));
    assert!(!marker.exists());
    let id = state
        .state()
        .pending_purge
        .as_ref()
        .unwrap()
        .event_id
        .clone();
    drop(state);
    drop(ledger);
    // 模拟删除后又出现的新残留；恢复审计绝不能碰它。
    fs::write(&marker, b"new").unwrap();
    let mut state = f.store();
    let report = recover(&f.ledger(), &mut state).unwrap().unwrap();
    assert_eq!(report["applied"], true);
    assert_eq!(report["removed_files"], 1);
    assert_eq!(fs::read(&marker).unwrap(), b"new");
    assert!(state.state().pending_purge.is_none());
    let records = f.records();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|r| r.event().event_id().to_string() == id)
    );
}

#[test]
fn interrupted_deletion_is_unknown_and_does_not_invent_zero_or_replay() {
    let f = Fixture::new();
    let pending = PendingPurge {
        event_id: "a".repeat(64),
        folder: "School".into(),
        planned_files: 3,
        planned_directories: 2,
        deletion_started: true,
        result: None,
    };
    let mut state = f.store();
    save(&mut state, Some(pending.clone())).unwrap();
    intent(&f.ledger(), &pending).unwrap();
    drop(state);
    let mut state = f.store();
    let report = recover(&f.ledger(), &mut state).unwrap().unwrap();
    assert_eq!(report["outcome_known"], false);
    assert!(report["applied"].is_null());
    assert!(report["removed_files"].is_null());
    let records = f.records();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].event().payload().as_value(),
        &json!({"schema_version":2,"folder":"School","files":null,"directories":null,"applied":null})
    );
    assert!(recover(&f.ledger(), &mut state).unwrap().is_none());
}

#[test]
fn partial_deletion_error_is_preserved_as_known_partial_counts() {
    let f = Fixture::new();
    let mut state = f.store();
    assert!(matches!(
        run(&f.ledger(), &mut state, "School", &planned(), |removed| {
            removed.files = 2;
            Err(MaintenanceError::SyncResidueUnsafe)
        }),
        Err(MaintenanceError::SyncResidueUnsafe)
    ));
    let records = f.records();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].event().payload().as_value(),
        &json!({"schema_version":1,"folder":"School","files":2,"directories":0,"applied":false})
    );
    assert!(state.state().pending_purge.is_none());
}

#[test]
fn pending_purge_is_checksum_protected_and_invalid_shape_is_refused() {
    let f = Fixture::new();
    let mut state = f.store();
    let mut pending = PendingPurge {
        event_id: "b".repeat(64),
        folder: "School".into(),
        planned_files: 3,
        planned_directories: 2,
        deletion_started: true,
        result: None,
    };
    pending.result = Some(PurgeResult {
        files: Some(1),
        directories: None,
        applied: Some(true),
    });
    assert!(save(&mut state, Some(pending.clone())).is_err());
    pending.result = None;
    save(&mut state, Some(pending)).unwrap();
    drop(state);
    let path = f.plane.join(crate::sync_plane::recovery::PLANE_STATE_FILE);
    let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["pending_purge"]["planned_files"] = json!(4);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(PlaneStateStore::open(&f.plane).is_err());
}
