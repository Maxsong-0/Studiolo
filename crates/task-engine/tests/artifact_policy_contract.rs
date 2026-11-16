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
    SourcePolicyRef, TaskId, VaultPath,
};
use studiolo_runner_adapter::{RunnerArtifact, RunnerReport};
use studiolo_task_engine::{
    ClaimRequest, PersistentTaskEngine, RetryPolicy, TaskCommand, TaskEngine, TaskEngineError,
    TaskSpec,
};
use studiolo_vault_broker::{MemoryVaultBroker, Mutation, VaultReader, VaultWriter};
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

#[test]
fn terminal_report_takes_provenance_from_the_approval_and_ledger() {
    let mut broker = MemoryVaultBroker::new();
    let vault = broker.vault_uuid();
    let input = VaultPath::parse("input.md").unwrap();
    broker
        .mutate(Mutation::create(
            input.clone(),
            b"input".to_vec(),
            "fixture",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let task = engine
        .create_task(
            TaskSpec {
                role: "implementer".into(),
                adapter: "stub".into(),
                input_paths: vec![input],
                allow_web: vec![],
                allow_db: vec![],
                retry_policy: RetryPolicy {
                    max_attempts: 1,
                    backoff_seconds: 0,
                    retry_on: vec![],
                },
                timeout: 300,
                project: "fixture".into(),
                parent: None,
                title: None,
                instruction: None,
            },
            1,
        )
        .unwrap();
    engine.command(task, TaskCommand::Run, 2).unwrap();
    let claimed = engine
        .claim(ClaimRequest {
            worker_id: "worker".into(),
            now_ms: 3,
            lease_until_ms: 100,
        })
        .unwrap()
        .unwrap();
    let report = RunnerReport {
        task_id: task,
        attempt_id: claimed.run_spec.attempt_id,
        fence: claimed.run_spec.fence,
        artifacts: vec![RunnerArtifact {
            name: VaultPath::parse("result.md").unwrap(),
            content: b"body".to_vec(),
        }],
    };
    assert!(matches!(
        engine.report("missing-approval", report.clone(), 4),
        Err(TaskEngineError::Integrity(ref message)) if message.contains("不能补成完整范围")
    ));
    let fact = ArtifactProvenance::new(
        SourcePolicyRef::new("fixture-source", "a".repeat(64)).unwrap(),
        DataLabels::new(
            Classification::Restricted,
            BackupScope::Full,
            DesktopSync::Deny,
            ModelEgress::Deny,
        )
        .unwrap(),
        4_000_000_000,
        "b".repeat(64),
    )
    .unwrap();
    engine
        .authorize_artifacts(
            task,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            &claimed.snapshot_id,
            &fact,
            4,
        )
        .unwrap();
    let committed = engine.report("approved-report", report.clone(), 5).unwrap();
    assert!(!committed.replayed);
    let manifest = engine
        .broker()
        .read(&committed.receipt.manifest_path)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&manifest.content).unwrap();
    assert_eq!(parsed["format_version"], 2);
    assert_eq!(parsed["provenance"], serde_json::to_value(&fact).unwrap());
    let replay = engine.report("approved-report", report, 6).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.receipt, committed.receipt);
}
