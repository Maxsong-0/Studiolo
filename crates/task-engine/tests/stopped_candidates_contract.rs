//! Cancelled runs retain a real stopped packet and recover without another CLI invocation.
mod stopped_candidates_support;
use stopped_candidates_support::*;
use studiolo_audit_ledger::{AuditFilter, AuditLedger, MemoryAuditLedger};
use studiolo_control_db::{ControlDb, TaskStatus};
use studiolo_core::Fence;
use studiolo_runner_adapter::RunnerArtifact;
use studiolo_task_engine::*;
use studiolo_vault_broker::*;

#[test]
fn stopped_publishes_patch_and_original_packet_without_passing_acceptance() {
    let (mut engine, task, claim) = claimed();
    engine.command(task, TaskCommand::Cancel, 2).unwrap();
    assert!(
        !engine
            .control_db()
            .is_attempt_active_at(task, claim.attempt.attempt_id, claim.attempt.fence, 3)
            .unwrap()
    );
    let mut report = report(&claim, b"diff --git a/src/a b/src/a\n+unfinished\n");
    report.artifacts.push(RunnerArtifact {
        name: path("stopped.json"),
        content: serde_json::to_vec(&stopped()).unwrap(),
    });
    let committed = engine
        .stage_board_stopped_candidates(report.clone(), stopped(), 3)
        .unwrap();
    let view = engine.board_detail(task, HASH).unwrap().unwrap();
    assert_eq!(view.task.status, TaskStatus::Cancelled);
    assert!(view.task.active_attempt.is_none());
    assert_eq!(view.task.last_attempt, Some(claim.attempt.attempt_id));
    let receipt: BoardReceipt = serde_json::from_value(view.board.receipt.unwrap()).unwrap();
    assert_eq!(receipt.status, ReceiptStatus::Stopped);
    assert_eq!(receipt.acceptance, stopped().acceptance);
    assert_eq!(receipt.artifacts.len(), 3);
    assert_eq!(receipt.unlanded.len(), 1);
    assert!(
        receipt.unlanded[0]
            .path
            .as_str()
            .ends_with("/candidate.patch")
    );
    let (_, broker, ledger) = engine.into_parts();
    let packet = committed
        .artifacts
        .iter()
        .find(|a| a.path.as_str().ends_with("/stopped.json"))
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<BoardReceipt>(&broker.read(&packet.path).unwrap().content)
            .unwrap(),
        stopped()
    );
    assert!(
        ledger
            .query(
                &AuditFilter::default()
                    .with_task(task)
                    .with_attempt(claim.attempt.attempt_id)
            )
            .unwrap()
            .iter()
            .any(|r| r.event().action() == "task.stopped_candidates_committed")
    );
}

#[test]
fn cancellation_rejects_success_direct_ai_return_wrong_fence_and_false_packet() {
    let (mut engine, task, claim) = claimed();
    engine.command(task, TaskCommand::Cancel, 2).unwrap();
    let report = report(&claim, b"diff --git a/src/a b/src/a\n+unfinished\n");
    assert!(
        engine
            .stage_board_runner_candidates(report.clone(), 3)
            .is_err()
    );
    assert!(
        engine
            .return_board_receipt(
                task,
                claim.attempt.attempt_id,
                claim.attempt.fence,
                stopped(),
                3
            )
            .is_err()
    );
    let mut wrong = report.clone();
    wrong.fence = Fence::new(claim.attempt.fence.get() + 1).unwrap();
    assert!(matches!(
        engine.stage_board_stopped_candidates(wrong, stopped(), 3),
        Err(TaskEngineError::StaleFence)
    ));
    let mut success = stopped();
    success.status = ReceiptStatus::Completed;
    assert!(
        engine
            .stage_board_stopped_candidates(report.clone(), success, 3)
            .is_err()
    );
    let mut passing = stopped();
    passing.acceptance[0].result = AssessmentResult::Pass;
    assert!(
        engine
            .stage_board_stopped_candidates(report.clone(), passing, 3)
            .is_err()
    );
    let mut mismatch = report.clone();
    mismatch.artifacts.push(RunnerArtifact {
        name: path("stopped.json"),
        content: b"{}".to_vec(),
    });
    assert!(
        engine
            .stage_board_stopped_candidates(mismatch, stopped(), 3)
            .is_err()
    );
    let mut conflict = stopped();
    conflict.conflicts.push(path("src/a"));
    assert!(
        engine
            .stage_board_stopped_candidates(report, conflict, 3)
            .is_err()
    );
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::CancelRequested
    );
}

#[test]
fn grace_expiry_retains_staging_then_abandons_instead_of_accepting_late_stopped() {
    let (mut engine, task, claim) = claimed();
    engine.command(task, TaskCommand::Cancel, 2).unwrap();
    assert!(
        engine
            .stage_board_stopped_candidates(
                report(&claim, b"diff --git a/src/a b/src/a\n+late\n"),
                stopped(),
                180_002
            )
            .is_err()
    );
    let view = engine.board_detail(task, HASH).unwrap().unwrap();
    assert_eq!(view.task.status, TaskStatus::CancelRequested);
    assert!(view.candidate_receipt.is_none());
    assert_eq!(view.candidate_stages.len(), 1);
    assert_eq!(view.candidate_stages[0].reason, "cancelled_or_inactive");
    engine.reconcile_periodic(180_002).unwrap();
    let view = engine.board_detail(task, HASH).unwrap().unwrap();
    assert_eq!(view.task.status, TaskStatus::Abandoned);
    assert_eq!(view.candidate_stages.len(), 1);
}

#[test]
fn stopped_manifest_ack_loss_recovers_after_grace_without_inline_bodies_or_cli() {
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
    let task = engine
        .create_board_task("stopped", spec(), HASH, 0)
        .unwrap();
    let claim = engine
        .claim_board_task(task, "runner-instance", 1000, 1)
        .unwrap();
    engine.command(task, TaskCommand::Cancel, 2).unwrap();
    let report = report(
        &claim,
        b"diff --git a/src/a b/src/a\n+PRIVATE_STOPPED_PATCH\n",
    );
    assert!(matches!(
        engine.stage_board_stopped_candidates(report.clone(), stopped(), 3),
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
        let parsed: serde_json::Value = serde_json::from_str(&reservation).unwrap();
        assert_eq!(parsed["report_json"]["outcome"], "stopped");
        for forbidden in [
            "PRIVATE_STOPPED_PATCH",
            "candidate.patch",
            "acceptance",
            "closeout",
            "content",
        ] {
            assert!(!reservation.contains(forbidden));
        }
        let receipt: Option<String> = connection
            .query_row("SELECT receipt_json FROM board_tasks", [], |r| r.get(0))
            .unwrap();
        assert!(receipt.is_none());
    }
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open(&database, vault).unwrap(),
        FsVaultBroker::open(&root, &data).unwrap(),
        ledger,
    );
    engine.reconcile_startup(180_003).unwrap();
    let view = engine.board_detail(task, HASH).unwrap().unwrap();
    assert_eq!(view.task.status, TaskStatus::Cancelled);
    let committed = view.candidate_receipt.unwrap();
    assert_eq!(
        engine
            .stage_board_stopped_candidates(report.clone(), stopped(), 180_004)
            .unwrap(),
        committed
    );
    let mut changed = stopped();
    changed.remaining.push("different receipt".into());
    assert!(matches!(
        engine.stage_board_stopped_candidates(report, changed, 180_005),
        Err(TaskEngineError::IdempotencyMismatch)
    ));
    let (db, _, _) = engine.into_parts();
    assert!(db.pending_terminal_audits().unwrap().is_empty());
}

#[test]
fn stopped_requires_cancellation_and_real_evidence_for_any_partial_pass() {
    let (mut engine, task, claim) = claimed();
    assert!(
        engine
            .stage_board_stopped_candidates(
                report(&claim, b"diff --git a/src/a b/src/a\n+early\n"),
                stopped(),
                2
            )
            .is_err()
    );
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Running
    );
    engine.command(task, TaskCommand::Cancel, 3).unwrap();
    let mut receipt = stopped();
    receipt.acceptance[0].evidence.push(ArtifactPointer {
        path: path("missing.txt"),
        revision: "a".repeat(64),
        sha256: "a".repeat(64),
    });
    assert!(
        engine
            .stage_board_stopped_candidates(
                report(&claim, b"diff --git a/src/a b/src/a\n+partial\n"),
                receipt,
                4
            )
            .is_err()
    );
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::CancelRequested
    );
}

#[test]
fn final_millisecond_of_cancel_window_can_publish_after_original_lease_expired() {
    let (mut engine, task, claim) = claimed();
    engine.command(task, TaskCommand::Cancel, 2).unwrap();
    engine
        .stage_board_stopped_candidates(
            report(&claim, b"diff --git a/src/a b/src/a\n+stopped\n"),
            stopped(),
            180_001,
        )
        .unwrap();
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Cancelled
    );
}
