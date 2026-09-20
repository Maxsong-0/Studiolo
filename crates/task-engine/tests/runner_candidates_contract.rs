//! Real Broker/SQLite candidate recovery and the separate owner approval boundary.
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, MemoryAuditLedger};
use studiolo_control_db::{ControlDb, TaskStatus};
use studiolo_core::{TaskId, VaultPath, VaultUuid};
use studiolo_runner_adapter::{PatchMetadata, RunnerArtifact, RunnerReport};
use studiolo_task_engine::*;
use studiolo_vault_broker::*;
const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}
fn spec() -> BoardTaskSpec {
    BoardTaskSpec {
        task: TaskSpec {
            role: "implementer".into(),
            adapter: "claude".into(),
            input_paths: vec![],
            allow_web: vec![],
            allow_db: vec![],
            retry_policy: RetryPolicy {
                max_attempts: 1,
                backoff_seconds: 0,
                retry_on: vec![],
            },
            timeout: 300,
            project: "review".into(),
            parent: None,
            title: None,
            instruction: None,
        },
        acceptance_criteria: vec!["owner verified the change".into()],
        files_owned: vec![path("src")],
        max_children: 8,
        artifact_target: Some("one implementation".into()),
    }
}
fn report(claim: &BoardClaim, patch: &[u8]) -> RunnerReport {
    let metadata = PatchMetadata {
        base_tree: "a".repeat(40),
        after_tree: "b".repeat(40),
        sha256: Sha256::digest(patch)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        retained_path: "/host-retained/candidate.patch".into(),
    };
    RunnerReport {
        task_id: claim.task.task.task_id,
        attempt_id: claim.attempt.attempt_id,
        fence: claim.attempt.fence,
        artifacts: vec![
            RunnerArtifact {
                name: path("candidate.patch"),
                content: patch.to_vec(),
            },
            RunnerArtifact {
                name: path("patch.json"),
                content: serde_json::to_vec(&metadata).unwrap(),
            },
        ],
    }
}
fn approval(candidates: &ArtifactCommitReceipt) -> BoardReceipt {
    BoardReceipt {
        status: ReceiptStatus::Completed,
        artifacts: candidates
            .artifacts
            .iter()
            .map(|a| ArtifactPointer {
                path: a.path.clone(),
                revision: a.revision.to_string(),
                sha256: a.sha256.clone(),
            })
            .collect(),
        conflicts: vec![],
        unlanded: vec![],
        acceptance: vec![AcceptanceAssessment {
            criterion: "owner verified the change".into(),
            result: AssessmentResult::Pass,
            evidence: vec![],
        }],
        remaining: vec![],
        elapsed_ms: 10,
        tool_calls: 2,
        learned: vec![],
        closeout: Closeout {
            learned: vec![],
            adr_required: false,
            rule_change_required: false,
            destinations: vec![],
        },
    }
}
fn claimed() -> (
    PersistentTaskEngine<MemoryVaultBroker, MemoryAuditLedger>,
    TaskId,
    BoardClaim,
) {
    let broker = MemoryVaultBroker::new();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(broker.vault_uuid()).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let task = engine.create_board_task("runner", spec(), HASH, 0).unwrap();
    let claim = engine
        .claim_board_task(task, "runner-instance", 1000, 1)
        .unwrap();
    (engine, task, claim)
}

struct LostManifestAck {
    inner: FsVaultBroker,
    fail: bool,
}
impl VaultReader for LostManifestAck {
    fn vault_uuid(&self) -> VaultUuid {
        self.inner.vault_uuid()
    }
    fn read(&self, p: &VaultPath) -> Result<VaultObject, BrokerError> {
        self.inner.read(p)
    }
}
impl VaultSnapshotter for LostManifestAck {
    fn snapshot(&self, i: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        self.inner.snapshot(i)
    }
}
impl SnapshotReader for LostManifestAck {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        self.inner.load_snapshot(id)
    }
}
impl ArtifactStager for LostManifestAck {
    fn stage_artifact_commit(
        &self,
        p: &ArtifactCommit,
    ) -> Result<StagedArtifactCommit, BrokerError> {
        self.inner.stage_artifact_commit(p)
    }
    fn load_staged_artifact_commit(
        &self,
        p: &StagedArtifactCommit,
    ) -> Result<ArtifactCommit, BrokerError> {
        self.inner.load_staged_artifact_commit(p)
    }
}
impl VaultWriter for LostManifestAck {
    fn mutate(&mut self, m: Mutation) -> Result<MutationReceipt, BrokerError> {
        self.inner.mutate(m)
    }
    fn commit_attempt_artifacts(
        &mut self,
        c: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        let receipt = self.inner.commit_attempt_artifacts(c)?;
        if std::mem::take(&mut self.fail) {
            Err(BrokerError::Io {
                operation: "lost manifest acknowledgement",
                source: std::io::Error::other("fixture crash"),
            })
        } else {
            Ok(receipt)
        }
    }
}

#[test]
fn manifest_ack_crash_recovers_candidates_into_needs_review_without_running_cli() {
    let temp = tempfile::TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let root = temp.path().join("vault");
    let data = temp.path().join("broker");
    let database = temp.path().join("control.sqlite3");
    let broker = FsVaultBroker::open(&root, &data).unwrap();
    let vault = broker.vault_uuid();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open(&database, vault).unwrap(),
        LostManifestAck {
            inner: broker,
            fail: true,
        },
        MemoryAuditLedger::new(),
    );
    let task = engine.create_board_task("runner", spec(), HASH, 0).unwrap();
    let claim = engine
        .claim_board_task(task, "runner-instance", 1000, 1)
        .unwrap();
    let report = report(&claim, b"diff --git a/src/a b/src/a\n+safe\n");
    assert!(matches!(
        engine.stage_board_runner_candidates(report.clone(), 2),
        Err(TaskEngineError::CommitFailed(_))
    ));
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Committing
    );
    let (db, broker, ledger) = engine.into_parts();
    drop(db);
    drop(broker);
    {
        let connection = rusqlite::Connection::open(&database).unwrap();
        let reservation: String = connection
            .query_row("SELECT terminal_json FROM attempts", [], |r| r.get(0))
            .unwrap();
        let pointer: String = connection
            .query_row(
                "SELECT pointer_json FROM runner_candidate_stages",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!reservation.contains("candidate.patch"));
        assert!(!reservation.contains("content"));
        assert!(!pointer.contains("content"));
    }
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open(&database, vault).unwrap(),
        FsVaultBroker::open(&root, &data).unwrap(),
        ledger,
    );
    engine.reconcile_startup(3).unwrap();
    let view = engine.board_detail(task, HASH).unwrap().unwrap();
    assert_eq!(view.task.status, TaskStatus::NeedsReview);
    assert!(view.task.active_attempt.is_none());
    assert_eq!(view.task.last_attempt, Some(claim.attempt.attempt_id));
    assert!(view.candidate_receipt.is_some());
    assert_eq!(
        view.attempt
            .unwrap()
            .terminal
            .unwrap()
            .artifacts_json
            .as_object()
            .unwrap()
            .len(),
        2
    );
    let replay = engine.stage_board_runner_candidates(report, 4).unwrap();
    assert_eq!(Some(replay), view.candidate_receipt);
}
#[test]
fn ai_cannot_self_approve_or_reclaim_candidates_awaiting_owner_review() {
    let (mut engine, task, claim) = claimed();
    let candidates = engine
        .stage_board_runner_candidates(report(&claim, b"diff --git a/src/a b/src/a\n+safe\n"), 2)
        .unwrap();
    assert!(
        engine
            .return_board_receipt(
                task,
                claim.attempt.attempt_id,
                claim.attempt.fence,
                approval(&candidates),
                3
            )
            .is_err()
    );
    assert!(
        engine
            .claim_board_task(task, "another-ai", 1000, 4)
            .is_err()
    );
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::NeedsReview
    );
}
#[test]
fn owner_must_verify_every_real_candidate_and_closeout_before_success() {
    let (mut engine, task, claim) = claimed();
    let candidates = engine
        .stage_board_runner_candidates(report(&claim, b"diff --git a/src/a b/src/a\n+safe\n"), 2)
        .unwrap();
    let mut missing = approval(&candidates);
    missing.artifacts.pop();
    assert!(engine.review_board_candidates(task, missing, 3).is_err());
    let mut unverified = approval(&candidates);
    unverified.acceptance[0].result = AssessmentResult::Unverified;
    assert!(engine.review_board_candidates(task, unverified, 4).is_err());
    let result = engine
        .review_board_candidates(task, approval(&candidates), 5)
        .unwrap();
    assert_eq!(result.task.status, TaskStatus::Succeeded);
    assert_eq!(result.candidate_receipt, Some(candidates));
    assert!(!result.candidate_stages.is_empty());
    let events = engine
        .audit_ledger()
        .query(
            &AuditFilter::default()
                .with_task(task)
                .with_attempt(claim.attempt.attempt_id),
        )
        .unwrap();
    assert!(
        events
            .iter()
            .any(|record| record.event().action() == "task.candidates_reviewed")
    );
    assert!(
        events
            .iter()
            .any(|record| record.event().action() == "task.runner_candidate_retained")
    );
}
#[test]
fn rejected_and_cancelled_candidates_keep_content_plane_staging_pointers() {
    let (mut engine, task, claim) = claimed();
    assert!(
        engine
            .stage_board_runner_candidates(
                report(&claim, b"+<<<<<<< ours\n+unresolved\n+>>>>>>> theirs\n"),
                2
            )
            .is_err()
    );
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Running
    );
    assert_eq!(
        engine
            .board_detail(task, HASH)
            .unwrap()
            .unwrap()
            .candidate_stages[0]
            .reason,
        "rejected"
    );
    engine.command(task, TaskCommand::Cancel, 3).unwrap();
    assert!(
        engine
            .stage_board_runner_candidates(
                report(&claim, b"diff --git a/src/a b/src/a\n+safe\n"),
                4
            )
            .is_err()
    );
    let view = engine.board_detail(task, HASH).unwrap().unwrap();
    assert_eq!(view.candidate_stages.len(), 2);
    assert!(
        view.candidate_stages
            .iter()
            .any(|s| s.reason == "cancelled_or_inactive")
    );
    assert_eq!(view.task.status, TaskStatus::CancelRequested);
}

#[test]
fn owner_review_rejects_candidates_changed_after_manifest_publication() {
    let (mut engine, task, claim) = claimed();
    let candidates = engine
        .stage_board_runner_candidates(report(&claim, b"diff --git a/src/a b/src/a\n+safe\n"), 2)
        .unwrap();
    let (db, mut broker, ledger) = engine.into_parts();
    let candidate = &candidates.artifacts[0];
    let object = broker.read(&candidate.path).unwrap();
    broker
        .mutate(Mutation::replace(
            candidate.path.clone(),
            b"changed candidate".to_vec(),
            object.revision,
            "out-of-band-fixture",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(db, broker, ledger);
    assert!(
        engine
            .review_board_candidates(task, approval(&candidates), 3)
            .is_err()
    );
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::NeedsReview
    );
}
