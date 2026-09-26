//! M5/M6 权限测试使用真实 SQLite，以及与生产相同的 `Broker`/审计接缝。
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::MemoryAuditLedger;
use studiolo_control_db::{ControlDb, TaskStatus};
use studiolo_core::{Fence, VaultPath, VaultUuid};
use studiolo_task_engine::*;
use studiolo_vault_broker::{MemoryVaultBroker, Mutation, VaultReader, VaultWriter};
const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
type Engine = PersistentTaskEngine<MemoryVaultBroker, MemoryAuditLedger>;
fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}
fn engine() -> Engine {
    let vault = VaultUuid::new();
    Engine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        MemoryVaultBroker::with_uuid(vault),
        MemoryAuditLedger::new(),
    )
}
fn spec(paths: &[&str]) -> BoardTaskSpec {
    BoardTaskSpec {
        task: TaskSpec {
            role: "implementer".into(),
            adapter: "consumer".into(),
            input_paths: vec![],
            allow_web: vec![],
            allow_db: vec![],
            retry_policy: RetryPolicy {
                max_attempts: 3,
                backoff_seconds: 1,
                retry_on: vec!["launch_failure".into()],
            },
            timeout: 300,
            project: "test".into(),
            parent: None,
            title: Some("contract".into()),
            instruction: None,
        },
        acceptance_criteria: vec!["the contract passes".into()],
        files_owned: paths.iter().map(|p| path(p)).collect(),
        max_children: 8,
        artifact_target: None,
    }
}
fn receipt() -> BoardReceipt {
    BoardReceipt {
        status: ReceiptStatus::Completed,
        artifacts: vec![],
        conflicts: vec![],
        unlanded: vec![],
        acceptance: vec![AcceptanceAssessment {
            criterion: "the contract passes".into(),
            result: AssessmentResult::Pass,
            evidence: vec![],
        }],
        remaining: vec![],
        elapsed_ms: 10,
        tool_calls: 1,
        learned: vec![],
        closeout: Closeout {
            learned: vec![],
            adr_required: false,
            rule_change_required: false,
            destinations: vec![],
        },
    }
}
#[test]
fn required_fields_hash_and_create_replay_are_machine_checked() {
    let mut engine = engine();
    let input = spec(&[]);
    let id = engine
        .create_board_task("create", input.clone(), HASH, 0)
        .unwrap();
    assert_eq!(
        engine
            .create_board_task("create", input.clone(), HASH, 1)
            .unwrap(),
        id
    );
    let mut changed = input.clone();
    changed.files_owned.push(path("src/a.rs"));
    assert!(
        engine
            .create_board_task("create", changed, HASH, 2)
            .is_err()
    );
    let mut value = serde_json::to_value(&input).unwrap();
    value.as_object_mut().unwrap().remove("files_owned");
    assert!(serde_json::from_value::<BoardTaskSpec>(value).is_err());
    let mut value = serde_json::to_value(&input).unwrap();
    value.as_object_mut().unwrap().remove("acceptance_criteria");
    assert!(serde_json::from_value::<BoardTaskSpec>(value).is_err());
    assert!(
        engine
            .board_detail(id, &"b".repeat(64))
            .unwrap()
            .unwrap()
            .uses_old_policy
    );
}
#[test]
fn directory_case_collision_and_component_boundaries() {
    let mut e = engine();
    let a = e.create_board_task("a", spec(&["Src"]), HASH, 0).unwrap();
    let b = e
        .create_board_task("b", spec(&["src/file.rs"]), HASH, 1)
        .unwrap();
    let c = e
        .create_board_task("c", spec(&["src-other/file.rs"]), HASH, 2)
        .unwrap();
    e.claim_board_task(a, "a", 1000, 10).unwrap();
    assert!(
        matches!(e.claim_board_task(b,"b",1000,11),Err(TaskEngineError::PathsHeld{by_instance,paths}) if by_instance=="a"&&paths==vec!["src/file.rs"])
    );
    e.claim_board_task(c, "c", 1000, 12).unwrap();
}
#[test]
fn hierarchy_checks_scope_siblings_fanout_and_parent_completion() {
    let mut e = engine();
    let mut parent = spec(&["src"]);
    parent.max_children = 1;
    let p = e.create_board_task("p", parent, HASH, 0).unwrap();
    let mut child = spec(&["elsewhere"]);
    child.task.parent = Some(p);
    assert!(e.create_board_task("bad", child.clone(), HASH, 1).is_err());
    child.files_owned = vec![path("src/a.rs")];
    let c = e.create_board_task("c", child.clone(), HASH, 1).unwrap();
    assert!(e.create_board_task("second", child, HASH, 2).is_err());
    assert!(e.claim_board_task(p, "parent", 1000, 10).is_err());
    let claim = e.claim_board_task(c, "child", 1000, 11).unwrap();
    e.return_board_receipt(
        c,
        claim.attempt.attempt_id,
        claim.attempt.fence,
        receipt(),
        12,
    )
    .unwrap();
    e.claim_board_task(p, "parent", 1000, 13).unwrap();
}
#[test]
fn depth_never_exceeds_three() {
    let mut e = engine();
    let mut parent = None;
    for depth in 0..5 {
        let mut s = spec(&[]);
        s.task.parent = parent;
        let result = e.create_board_task(&format!("d{depth}"), s, HASH, depth);
        if depth == 4 {
            assert!(result.is_err());
        } else {
            parent = Some(result.unwrap());
        }
    }
}
#[test]
fn receipt_requires_all_criteria_closeout_and_matching_fence() {
    let mut e = engine();
    let id = e.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let a = e.claim_board_task(id, "worker", 1000, 1).unwrap().attempt;
    let mut invalid = receipt();
    invalid.acceptance.clear();
    assert!(
        e.return_board_receipt(id, a.attempt_id, a.fence, invalid, 2)
            .is_err()
    );
    let mut invalid = receipt();
    invalid.conflicts.push(path("src/a.rs"));
    assert!(
        e.return_board_receipt(id, a.attempt_id, a.fence, invalid, 2)
            .is_err()
    );
    let mut value = serde_json::to_value(receipt()).unwrap();
    value.as_object_mut().unwrap().remove("closeout");
    assert!(serde_json::from_value::<BoardReceipt>(value).is_err());
    assert!(
        e.return_board_receipt(
            id,
            a.attempt_id,
            Fence::new(a.fence.get() + 1).unwrap(),
            receipt(),
            2
        )
        .is_err()
    );
    assert_eq!(
        e.return_board_receipt(id, a.attempt_id, a.fence, receipt(), 2)
            .unwrap()
            .task
            .status,
        TaskStatus::Succeeded
    );
}
#[test]
fn first_valid_artifact_target_wins_after_failed_candidate_does_not() {
    let mut e = engine();
    let mut s = spec(&[]);
    s.artifact_target = Some("one implementation".into());
    let a = e.create_board_task("a", s.clone(), HASH, 0).unwrap();
    let b = e.create_board_task("b", s, HASH, 0).unwrap();
    let ca = e.claim_board_task(a, "a", 1000, 1).unwrap().attempt;
    let cb = e.claim_board_task(b, "b", 1000, 1).unwrap().attempt;
    let mut invalid = receipt();
    invalid.acceptance[0].result = AssessmentResult::Unverified;
    assert!(
        e.return_board_receipt(a, ca.attempt_id, ca.fence, invalid, 2)
            .is_err()
    );
    e.return_board_receipt(b, cb.attempt_id, cb.fence, receipt(), 3)
        .unwrap();
    assert!(
        e.return_board_receipt(a, ca.attempt_id, ca.fence, receipt(), 4)
            .is_err()
    );
}
#[test]
fn healthy_leases_survive_periodic_sweep_stalls_and_expiry_do_not() {
    let mut e = engine();
    let id = e.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let a = e
        .claim_board_task(id, "worker", 1_000_000, 1)
        .unwrap()
        .attempt;
    assert!(e.reconcile_periodic(500_000).unwrap().abandoned.is_empty());
    e.heartbeat_progress(id, a.attempt_id, a.fence, 1_000_000, 1, 0, 500_001)
        .unwrap();
    assert!(
        e.heartbeat_progress(id, a.attempt_id, a.fence, 1_000_000, 0, 0, 500_002)
            .is_err()
    );
    assert!(e.reconcile_periodic(600_002).unwrap().abandoned.is_empty());
    assert_eq!(
        e.reconcile_periodic(1_000_000).unwrap().abandoned[0].2,
        "lease_expired"
    );
    assert!(
        e.heartbeat_progress(id, a.attempt_id, a.fence, 2_000_000, 1, 0, 1_000_001)
            .is_err()
    );
    let mut stalled = spec(&[]);
    stalled.task.adapter = "stub".into();
    let id = e
        .create_board_task("stalled", stalled, HASH, 2_000_000)
        .unwrap();
    e.claim_board_task(id, "w", 4_000_000, 2_000_001).unwrap();
    assert_eq!(
        e.reconcile_periodic(2_600_001).unwrap().abandoned[0].2,
        "stalled"
    );
}
#[test]
fn cancel_immediately_blocks_guard_but_accepts_only_stopped_during_grace() {
    let mut e = engine();
    let id = e.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let a = e
        .claim_board_task(id, "worker", 1_000_000, 1)
        .unwrap()
        .attempt;
    e.command(id, TaskCommand::Cancel, 2).unwrap();
    assert!(
        !e.owns_board_claim(id, a.attempt_id, a.fence, "worker", 3)
            .unwrap()
    );
    assert!(
        e.return_board_receipt(id, a.attempt_id, a.fence, receipt(), 3)
            .is_err()
    );
    let mut stopped = receipt();
    stopped.status = ReceiptStatus::Stopped;
    assert_eq!(
        e.return_board_receipt(id, a.attempt_id, a.fence, stopped, 180_001)
            .unwrap()
            .task
            .status,
        TaskStatus::Cancelled
    );
    let id = e
        .create_board_task("expired", spec(&[]), HASH, 200_000)
        .unwrap();
    e.claim_board_task(id, "worker", 1_000_000, 200_001)
        .unwrap();
    e.command(id, TaskCommand::Cancel, 200_002).unwrap();
    assert_eq!(
        e.reconcile_periodic(380_002).unwrap().abandoned[0].2,
        "cancel_timeout"
    );
}
#[test]
fn number_uses_real_filename_max_and_durable_reservations() {
    let vault = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault);
    broker
        .mutate(Mutation::create(
            path("notes/ADR-009-last.md"),
            b"x".to_vec(),
            "fixture",
        ))
        .unwrap();
    let mut e = Engine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let id = e.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    assert_eq!(
        e.reserve_number("n1", id, &path("notes"), "ADR-").unwrap(),
        10
    );
    assert_eq!(
        e.reserve_number("n1", id, &path("notes"), "ADR-").unwrap(),
        10
    );
    assert_eq!(
        e.reserve_number("n2", id, &path("notes"), "ADR-").unwrap(),
        11
    );
}
#[test]
fn forged_revision_hash_and_conflict_markers_fail_before_terminal() {
    let vault = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault);
    let p = path("patch.md");
    broker
        .mutate(Mutation::create(
            p.clone(),
            b"<<<<<<< ours\nconflict\n>>>>>>> theirs\n".to_vec(),
            "fixture",
        ))
        .unwrap();
    let object = broker.read(&p).unwrap();
    let mut e = Engine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let id = e.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let a = e.claim_board_task(id, "worker", 1000, 1).unwrap().attempt;
    let mut report = receipt();
    report.artifacts.push(ArtifactPointer {
        path: p,
        revision: object.revision.to_string(),
        sha256: Sha256::digest(&object.content)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    });
    assert!(
        e.return_board_receipt(id, a.attempt_id, a.fence, report, 2)
            .is_err()
    );
    assert_eq!(e.task(id).unwrap().unwrap().status, TaskStatus::Running);
}
#[test]
fn launch_failure_retries_twice_with_backoff_and_new_fences() {
    let mut e = engine();
    let id = e.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let first = e.claim_board_task(id, "worker", 10000, 1).unwrap().attempt;
    assert!(
        e.launch_failed(id, first.attempt_id, first.fence, 2)
            .unwrap()
    );
    assert!(e.claim_board_task(id, "worker", 10000, 3).is_err());
    let second = e
        .claim_board_task(id, "worker", 10000, 1002)
        .unwrap()
        .attempt;
    assert!(second.fence.get() > first.fence.get());
    assert!(
        e.launch_failed(id, second.attempt_id, second.fence, 1003)
            .unwrap()
    );
    let third = e
        .claim_board_task(id, "worker", 10000, 2003)
        .unwrap()
        .attempt;
    assert!(
        !e.launch_failed(id, third.attempt_id, third.fence, 2004)
            .unwrap()
    );
    assert_eq!(e.task(id).unwrap().unwrap().status, TaskStatus::Failed);
}

#[test]
fn structured_handoff_checks_trusted_rules_and_file_revisions() {
    let mut e = engine();
    let id = e.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let a = e.claim_board_task(id, "worker", 1000, 1).unwrap().attempt;
    assert!(serde_json::from_value::<Handoff>(serde_json::json!("go do it")).is_err());
    let mut handoff = Handoff {
        goal: "finish the bounded task".into(),
        constraints: vec![RuleConstraint {
            rule_id: "project-rules".into(),
            version_hash: "b".repeat(64),
        }],
        decided: vec![],
        undecided: vec![],
        required_files: vec![],
        acceptance_criteria: vec!["the contract passes".into()],
        forbidden: vec!["no unrelated edits".into()],
    };
    assert!(
        e.put_handoff(id, a.attempt_id, a.fence, handoff.clone(), 2)
            .is_err()
    );
    handoff.constraints[0].version_hash = HASH.into();
    e.put_handoff(id, a.attempt_id, a.fence, handoff.clone(), 3)
        .unwrap();
    assert_eq!(
        e.board_detail(id, HASH)
            .unwrap()
            .unwrap()
            .board
            .handoff
            .unwrap(),
        serde_json::to_value(handoff).unwrap()
    );
}
#[test]
fn generated_board_block_preserves_human_rows_and_unknown_frontmatter() {
    let vault = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault);
    let p = path("project/TASKS.md");
    let text = b"---\ncustom: keep\n---\n| Human | do not overwrite |\n";
    broker
        .mutate(Mutation::create(p.clone(), text.to_vec(), "fixture"))
        .unwrap();
    let mut e = Engine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    e.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    e.project_board(&p, "test", HASH).unwrap();
    let first = e.broker().read(&p).unwrap();
    assert!(first.content.starts_with(text));
    e.project_board(&p, "test", HASH).unwrap();
    assert_eq!(e.broker().read(&p).unwrap().content, first.content);
}
#[test]
fn unavailable_audit_prevents_board_authority_changes() {
    use studiolo_audit_ledger::{AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord};
    struct Offline;
    impl AuditLedger for Offline {
        fn append(&self, _: AuditEvent) -> Result<AuditRecord, AuditError> {
            Err(AuditError::Unavailable)
        }
        fn query(&self, _: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
            Ok(vec![])
        }
    }
    let vault = VaultUuid::new();
    let mut e = PersistentTaskEngine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        MemoryVaultBroker::with_uuid(vault),
        Offline,
    );
    assert!(matches!(
        e.create_board_task("t", spec(&[]), HASH, 0),
        Err(TaskEngineError::AuditUnavailable)
    ));
    assert!(e.control_db().board_tasks().unwrap().is_empty());
}

#[test]
fn handoff_releases_paths_and_next_holder_gets_a_new_fence() {
    let mut e = engine();
    let id = e.create_board_task("t", spec(&["src"]), HASH, 0).unwrap();
    let a = e.claim_board_task(id, "first", 1000, 1).unwrap().attempt;
    let handoff = Handoff {
        goal: "finish the bounded task".into(),
        constraints: vec![RuleConstraint {
            rule_id: "project-rules".into(),
            version_hash: HASH.into(),
        }],
        decided: vec![],
        undecided: vec![],
        required_files: vec![],
        acceptance_criteria: vec!["the contract passes".into()],
        forbidden: vec!["no unrelated edits".into()],
    };
    e.put_handoff(id, a.attempt_id, a.fence, handoff, 2)
        .unwrap();
    assert_eq!(e.task(id).unwrap().unwrap().status, TaskStatus::Queued);
    assert!(
        !e.owns_board_claim(id, a.attempt_id, a.fence, "first", 3)
            .unwrap()
    );
    let b = e.claim_board_task(id, "second", 1000, 3).unwrap().attempt;
    assert!(b.fence.get() > a.fence.get());
    assert!(
        e.return_board_receipt(id, a.attempt_id, a.fence, receipt(), 4)
            .is_err()
    );
    e.return_board_receipt(id, b.attempt_id, b.fence, receipt(), 4)
        .unwrap();
    assert!(
        e.return_board_receipt(id, a.attempt_id, a.fence, receipt(), 5)
            .is_err()
    );
}
#[test]
fn manual_change_inside_generated_block_is_preserved_and_audited() {
    use studiolo_audit_ledger::{AuditFilter, AuditLedger};
    let mut e = engine();
    let id = e.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let p = path("project/TASKS.md");
    e.project_board(&p, "test", HASH).unwrap();
    let (db, mut broker, ledger) = e.into_parts();
    let before = broker.read(&p).unwrap();
    let edited = String::from_utf8(before.content)
        .unwrap()
        .replace("| draft |", "| human override |PRIVATE-EDIT|");
    broker
        .mutate(Mutation::replace(
            p.clone(),
            edited.into_bytes(),
            before.revision,
            "human-edit",
        ))
        .unwrap();
    let mut e = Engine::new(db, broker, ledger);
    e.claim_board_task(id, "worker", 1000, 1).unwrap();
    e.project_board(&p, "test", HASH).unwrap();
    let projected = String::from_utf8(e.broker().read(&p).unwrap().content).unwrap();
    assert!(projected.contains("PRIVATE-EDIT"));
    assert!(projected.contains("保留的带外黑板修改"));
    assert_eq!(
        projected.matches("<!-- studiolo:board:start -->").count(),
        1
    );
    assert!(projected.contains("| running |"));
    assert!(
        e.audit_ledger()
            .query(&AuditFilter::default())
            .unwrap()
            .iter()
            .any(|record| record.event().action() == "task.board_out_of_band_preserved")
    );
    let once = projected;
    e.project_board(&p, "test", HASH).unwrap();
    assert_eq!(
        String::from_utf8(e.broker().read(&p).unwrap().content).unwrap(),
        once
    );
    assert!(e.control_db().pending_outbox().unwrap().is_empty());
}
