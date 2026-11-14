//! 原批准先持久再投递审计；审计中断后重放同一批准，不得据此扩大范围。
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord, MemoryAuditLedger,
};
use studiolo_control_db::{ControlDb, SnapshotRecord};
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef, TaskId,
};
use studiolo_task_engine::{PersistentTaskEngine, TaskEngineError};
use studiolo_vault_broker::{MemoryVaultBroker, VaultReader};
#[derive(Clone)]
struct Switch {
    ledger: Arc<MemoryAuditLedger>,
    fail: Arc<AtomicBool>,
}
impl AuditLedger for Switch {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.action() == "task.artifacts.authorized" && self.fail.load(Ordering::SeqCst) {
            return Err(AuditError::Unavailable);
        }
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}
#[test]
fn artifact_approval_cannot_report_ready_until_original_ledger_acknowledges() {
    let broker = MemoryVaultBroker::new();
    let vault = broker.vault_uuid();
    let mut db = ControlDb::open_in_memory(vault).unwrap();
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
    let ledger = Switch {
        ledger: Arc::new(MemoryAuditLedger::new()),
        fail: Arc::new(AtomicBool::new(true)),
    };
    let mut engine = PersistentTaskEngine::new(db, broker, ledger.clone());
    let fact = ArtifactProvenance::new(
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
    assert!(matches!(
        engine.authorize_artifacts(task, a.attempt_id, a.fence, &a.snapshot_id, &fact, 4),
        Err(TaskEngineError::AuditUnavailable)
    ));
    assert_eq!(
        engine
            .control_db()
            .artifact_provenance(task, a.attempt_id, a.fence)
            .unwrap(),
        Some(fact.clone())
    );
    assert_eq!(
        engine.control_db().pending_runtime_audits().unwrap().len(),
        1
    );
    assert!(
        engine
            .control_db()
            .attempt(task, a.attempt_id)
            .unwrap()
            .unwrap()
            .runner_ref_json
            .is_none()
    );
    ledger.fail.store(false, Ordering::SeqCst);
    let (db, broker, ledger) = engine.into_parts();
    let mut restarted = PersistentTaskEngine::new(db, broker, ledger);
    restarted
        .authorize_artifacts(task, a.attempt_id, a.fence, &a.snapshot_id, &fact, 5)
        .unwrap();
    assert!(
        restarted
            .control_db()
            .pending_runtime_audits()
            .unwrap()
            .is_empty()
    );
    let records = restarted
        .audit_ledger()
        .query(&AuditFilter::default())
        .unwrap();
    let approved: Vec<_> = records
        .iter()
        .filter(|r| r.event().action() == "task.artifacts.authorized")
        .collect();
    assert_eq!(approved.len(), 1);
    assert_eq!(
        approved[0].event().payload().as_value()["approved_at_ms"],
        4
    );
}
