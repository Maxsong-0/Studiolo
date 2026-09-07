//! Public-seam contract tests for the persistent TaskEngine.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};
use studiolo_control_db::{AttemptPhase, ControlDb, TaskStatus};
use studiolo_core::{Fence, VaultPath, VaultUuid};
use studiolo_runner_adapter::{
    RunnerAdapter, RunnerArtifact, RunnerObservation, RunnerReport, StubRunner,
};
use studiolo_task_engine::{
    ClaimRequest, ClaimedTask, PersistentTaskEngine, RetryPolicy, TaskCommand, TaskEngine,
    TaskEngineError, TaskSpec,
};
use studiolo_vault_broker::{
    ArtifactCommit, ArtifactCommitReceipt, BrokerError, FsVaultBroker, MemoryVaultBroker, Mutation,
    MutationReceipt, Snapshot, SnapshotId, SnapshotInput, SnapshotReader, VaultObject, VaultReader,
    VaultSnapshotter, VaultWriter,
};
use tempfile::TempDir;

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("fixture path is valid")
}

fn spec(input: VaultPath) -> TaskSpec {
    TaskSpec {
        role: "implementer".to_owned(),
        adapter: "stub".to_owned(),
        input_paths: vec![input],
        allow_web: Vec::new(),
        allow_db: Vec::new(),
        retry_policy: RetryPolicy {
            max_attempts: 1,
            backoff_seconds: 0,
            retry_on: Vec::new(),
        },
        timeout: 300,
        project: "Studiolo".to_owned(),
        parent: None,
        title: Some("完成 M0".to_owned()),
        instruction: Some("总结输入并生成结果".to_owned()),
    }
}

fn accepts_object_safe_seam(_engine: &mut dyn TaskEngine) {}

#[derive(Clone)]
struct OutcomeSwitchLedger {
    inner: Arc<MemoryAuditLedger>,
    fail_outcome: Arc<AtomicBool>,
    fail_intent: Arc<AtomicBool>,
    outcome_appends: Arc<AtomicUsize>,
}

impl OutcomeSwitchLedger {
    fn new(fail_outcome: bool) -> Self {
        Self {
            inner: Arc::new(MemoryAuditLedger::new()),
            fail_outcome: Arc::new(AtomicBool::new(fail_outcome)),
            fail_intent: Arc::new(AtomicBool::new(false)),
            outcome_appends: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn allow_outcome(&self) {
        self.fail_outcome.store(false, Ordering::SeqCst);
    }
}

impl AuditLedger for OutcomeSwitchLedger {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.phase() == AuditPhase::Outcome {
            self.outcome_appends.fetch_add(1, Ordering::SeqCst);
        }
        if event.phase() == AuditPhase::Outcome && self.fail_outcome.load(Ordering::SeqCst) {
            return Err(AuditError::Unavailable);
        }
        if event.phase() == AuditPhase::Intent && self.fail_intent.load(Ordering::SeqCst) {
            return Err(AuditError::Unavailable);
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

#[derive(Clone, Copy)]
enum CrashPoint {
    BeforeArtifacts,
    AfterFirstArtifact,
    AfterManifest,
}

struct InterruptingBroker {
    inner: FsVaultBroker,
    crash: Option<CrashPoint>,
    commits: Arc<AtomicUsize>,
}

impl VaultReader for InterruptingBroker {
    fn vault_uuid(&self) -> VaultUuid {
        self.inner.vault_uuid()
    }
    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        self.inner.read(path)
    }
}

impl VaultSnapshotter for InterruptingBroker {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        self.inner.snapshot(inputs)
    }
}

impl SnapshotReader for InterruptingBroker {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        self.inner.load_snapshot(id)
    }
}

impl VaultWriter for InterruptingBroker {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError> {
        self.inner.mutate(mutation)
    }

    fn commit_attempt_artifacts(
        &mut self,
        commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        self.commits.fetch_add(1, Ordering::SeqCst);
        let Some(crash) = self.crash.take() else {
            return self.inner.commit_attempt_artifacts(commit);
        };
        match crash {
            CrashPoint::BeforeArtifacts => {}
            CrashPoint::AfterFirstArtifact => {
                let artifact = &commit.artifacts[0];
                self.inner.mutate(Mutation::create(
                    path(&format!(
                        "sources/attempts/{}/{}/{}",
                        commit.task_id, commit.attempt_id, artifact.name
                    )),
                    artifact.content.clone(),
                    "fault-injection-first-artifact",
                ))?;
            }
            CrashPoint::AfterManifest => {
                self.inner.commit_attempt_artifacts(commit)?;
            }
        }
        Err(BrokerError::Io {
            operation: "injected commit interruption",
            source: std::io::Error::other("test interruption"),
        })
    }
}

#[test]
fn restart_rolls_forward_no_partial_and_complete_manifest_without_runner_reexecution() {
    for crash in [
        CrashPoint::BeforeArtifacts,
        CrashPoint::AfterFirstArtifact,
        CrashPoint::AfterManifest,
    ] {
        let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let root = temp.path().join("vault");
        let data = temp.path().join("data");
        let db_path = data.join("control.sqlite3");
        let mut inner = FsVaultBroker::open(&root, &data).unwrap();
        let vault = inner.vault_uuid();
        inner
            .mutate(Mutation::create(
                path("input.md"),
                b"frozen".to_vec(),
                "fixture",
            ))
            .unwrap();
        let ledger = OutcomeSwitchLedger::new(false);
        let calls = Arc::new(AtomicUsize::new(0));
        let broker = InterruptingBroker {
            inner,
            crash: Some(crash),
            commits: calls.clone(),
        };
        let mut engine = PersistentTaskEngine::new(
            ControlDb::open(&db_path, vault).unwrap(),
            broker,
            ledger.clone(),
        );
        let task = engine.create_task(spec(path("input.md")), 1).unwrap();
        engine.command(task, TaskCommand::Run, 2).unwrap();
        let claimed = engine
            .claim(ClaimRequest {
                worker_id: "worker".to_owned(),
                now_ms: 3,
                lease_until_ms: 100,
            })
            .unwrap()
            .unwrap();
        let mut report = report_for(&claimed, b"one");
        report.artifacts.push(RunnerArtifact {
            name: path("second.md"),
            content: b"two".to_vec(),
        });
        assert!(matches!(
            engine.report("crash-matrix", report.clone(), 4),
            Err(TaskEngineError::CommitFailed(_))
        ));
        assert_eq!(
            engine.task(task).unwrap().unwrap().status,
            TaskStatus::Committing
        );
        assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 1);
        let manifest_path = path(&format!(
            "sources/attempts/{}/{}/manifest.json",
            task, claimed.run_spec.attempt_id
        ));
        let before = engine.broker().read(&manifest_path).ok();
        assert_eq!(before.is_some(), matches!(crash, CrashPoint::AfterManifest));
        drop(engine);

        let broker = InterruptingBroker {
            inner: FsVaultBroker::open(&root, &data).unwrap(),
            crash: None,
            commits: calls.clone(),
        };
        let mut restarted = PersistentTaskEngine::new(
            ControlDb::open(&db_path, vault).unwrap(),
            broker,
            ledger.clone(),
        );
        let summary = restarted.reconcile(20).unwrap();
        assert_eq!(summary.committed_attempts, 1);
        assert_eq!(summary.abandoned_attempts, 0);
        assert_eq!(
            restarted.task(task).unwrap().unwrap().status,
            TaskStatus::NeedsReview
        );
        let manifest = restarted.broker().read(&manifest_path).unwrap();
        if let Some(before) = before {
            assert_eq!(manifest, before);
        }
        let committed_calls = calls.load(Ordering::SeqCst);
        assert_eq!(committed_calls, 2);
        assert!(
            restarted
                .report("crash-matrix", report, 21)
                .unwrap()
                .replayed
        );
        assert_eq!(restarted.reconcile(22).unwrap().committed_attempts, 0);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            committed_calls,
            "completed replay never re-enters Broker commit"
        );
        assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
        assert!(
            restarted
                .control_db()
                .pending_terminal_audits()
                .unwrap()
                .is_empty()
        );
        let entries = std::fs::read_dir(root.join(format!(
            "sources/attempts/{}/{}/",
            task, claimed.run_spec.attempt_id
        )))
        .unwrap();
        assert_eq!(entries.count(), 3);
    }
}

#[test]
fn cancellation_wins_before_report_and_acknowledgement_preserves_the_terminal_state() {
    let (mut engine, claimed) = claimed_memory_engine();
    let task = claimed.run_spec.task_id;
    let cancelled = engine.command(task, TaskCommand::Cancel, 10).unwrap();
    assert_eq!(cancelled.status, TaskStatus::CancelRequested);
    assert_eq!(
        engine.command(task, TaskCommand::Cancel, 11).unwrap(),
        cancelled
    );
    assert!(
        !engine
            .control_db()
            .is_attempt_active(task, claimed.run_spec.attempt_id, claimed.run_spec.fence)
            .unwrap()
    );
    assert!(matches!(
        engine.report("late", report_for(&claimed, b"late"), 12),
        Err(TaskEngineError::AlreadyTerminal)
    ));
    assert!(matches!(
        engine.heartbeat(
            task,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            200,
            13
        ),
        Err(TaskEngineError::StaleFence)
    ));
    assert!(matches!(
        engine.acknowledge_cancel(
            task,
            claimed.run_spec.attempt_id,
            Fence::new(claimed.run_spec.fence.get() + 1).unwrap(),
            14
        ),
        Err(TaskEngineError::StaleFence)
    ));
    let terminal = engine
        .acknowledge_cancel(
            task,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            15,
        )
        .unwrap();
    assert_eq!(terminal.status, TaskStatus::Cancelled);
    assert_eq!(terminal.active_attempt, None);
    assert_eq!(terminal.last_attempt, Some(claimed.run_spec.attempt_id));
    assert_eq!(
        engine
            .control_db()
            .attempt(task, claimed.run_spec.attempt_id)
            .unwrap()
            .unwrap()
            .phase,
        AttemptPhase::Cancelled
    );
    assert_eq!(engine.reconcile(20).unwrap().abandoned_attempts, 0);
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Cancelled
    );
    assert!(
        engine
            .audit_ledger()
            .query(&AuditFilter::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn completed_report_wins_before_cancellation() {
    let (mut engine, claimed) = claimed_memory_engine();
    engine
        .report("winner", report_for(&claimed, b"complete"), 10)
        .unwrap();
    assert!(matches!(
        engine.command(claimed.run_spec.task_id, TaskCommand::Cancel, 11),
        Err(TaskEngineError::AlreadyTerminal)
    ));
    assert_eq!(
        engine
            .task(claimed.run_spec.task_id)
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::NeedsReview
    );
}

#[test]
fn audit_intent_failure_stops_artifacts_and_reconcile_can_retry_the_reserved_plan() {
    let (engine, claimed) = claimed_memory_engine();
    let (db, broker, _) = engine.into_parts();
    let ledger = OutcomeSwitchLedger::new(false);
    ledger.fail_intent.store(true, Ordering::SeqCst);
    let mut engine = PersistentTaskEngine::new(db, broker, ledger.clone());
    assert!(matches!(
        engine.report("intent-unavailable", report_for(&claimed, b"protected"), 10),
        Err(TaskEngineError::AuditUnavailable)
    ));
    let manifest = path(&format!(
        "sources/attempts/{}/{}/manifest.json",
        claimed.run_spec.task_id, claimed.run_spec.attempt_id
    ));
    assert!(matches!(
        engine.broker().read(&manifest),
        Err(BrokerError::NotFound(_))
    ));
    assert!(ledger.query(&AuditFilter::default()).unwrap().is_empty());
    assert_eq!(
        engine
            .task(claimed.run_spec.task_id)
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Committing
    );
    assert!(matches!(
        engine.command(claimed.run_spec.task_id, TaskCommand::Cancel, 11),
        Err(TaskEngineError::AlreadyTerminal)
    ));
    ledger.fail_intent.store(false, Ordering::SeqCst);
    assert_eq!(engine.reconcile(20).unwrap().committed_attempts, 1);
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 2);
}

#[test]
fn explicit_input_change_after_run_requires_reconfirmation_without_creating_attempt() {
    let vault = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault);
    let input = path("input.md");
    broker
        .mutate(Mutation::create(
            input.clone(),
            b"confirmed".to_vec(),
            "initial",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let task = engine.create_task(spec(input.clone()), 1).unwrap();
    let queued = engine.command(task, TaskCommand::Run, 2).unwrap();
    let (db, mut broker, ledger) = engine.into_parts();
    let before = broker.read(&input).unwrap();
    broker
        .mutate(Mutation::replace(
            input,
            b"changed".to_vec(),
            before.revision,
            "human-edit",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(db, broker, ledger);
    assert_eq!(
        engine
            .command(task, TaskCommand::Run, 3)
            .unwrap()
            .input_snapshot,
        queued.input_snapshot
    );
    assert!(
        matches!(engine.claim(ClaimRequest { worker_id: "worker".to_owned(), now_ms: 4, lease_until_ms: 100 }), Err(TaskEngineError::NeedsReconfirmation(id)) if id == task)
    );
    let view = engine.task(task).unwrap().unwrap();
    assert_eq!(view.status, TaskStatus::NeedsReconfirm);
    assert_eq!(view.active_attempt, None);
    assert_eq!(view.input_snapshot, queued.input_snapshot);
}

#[test]
fn engine_owned_page_projection_does_not_invalidate_the_confirmed_run_snapshot() {
    let vault = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault);
    broker
        .mutate(Mutation::create(
            path("input.md"),
            b"confirmed".to_vec(),
            "initial",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let task = engine.create_task(spec(path("input.md")), 1).unwrap();
    let queued = engine.command(task, TaskCommand::Run, 2).unwrap();
    engine.project_pending().unwrap();
    let claimed = engine
        .claim(ClaimRequest {
            worker_id: "worker".to_owned(),
            now_ms: 3,
            lease_until_ms: 100,
        })
        .unwrap()
        .unwrap();
    assert_eq!(Some(claimed.snapshot_id), queued.input_snapshot);
    let page = claimed
        .run_spec
        .inputs
        .iter()
        .find(|input| input.path.to_string() == format!("ops/tasks/{task}.md"))
        .unwrap();
    assert!(
        std::str::from_utf8(&page.content)
            .unwrap()
            .contains("status: \"draft\""),
        "runner retains exactly the page confirmed at Run"
    );
}

#[test]
fn broker_from_another_vault_cannot_supply_a_run_snapshot() {
    let mut broker = MemoryVaultBroker::new();
    broker
        .mutate(Mutation::create(
            path("input.md"),
            b"input".to_vec(),
            "initial",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(VaultUuid::new()).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let task = engine.create_task(spec(path("input.md")), 1).unwrap();
    assert!(matches!(
        engine.command(task, TaskCommand::Run, 2),
        Err(TaskEngineError::Integrity(_))
    ));
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Draft
    );
}

#[test]
fn runner_reference_cannot_bypass_pid_start_identity_validation() {
    let (mut engine, claimed) = claimed_memory_engine();
    let mut forged = studiolo_runner_adapter::RunnerRef::opaque("stub", "forged");
    forged.pid = Some(100);
    assert!(matches!(
        engine.attach_runner(
            claimed.run_spec.task_id,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            forged,
            10
        ),
        Err(TaskEngineError::InvalidRunnerRef)
    ));
    assert!(
        engine
            .control_db()
            .attempt(claimed.run_spec.task_id, claimed.run_spec.attempt_id)
            .unwrap()
            .unwrap()
            .runner_ref_json
            .is_none()
    );
}

#[test]
fn m0_refuses_external_adapters_and_process_backed_stub_references() {
    let vault = VaultUuid::new();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        MemoryVaultBroker::with_uuid(vault),
        MemoryAuditLedger::new(),
    );
    let mut external = spec(path("input.md"));
    external.adapter = "codex".to_owned();
    assert!(matches!(
        engine.create_task(external, 1),
        Err(TaskEngineError::InvalidTaskSpec(_))
    ));
    let (mut engine, claimed) = claimed_memory_engine();
    let process_ref = studiolo_runner_adapter::RunnerRef::process("stub", 123, 456).unwrap();
    assert!(matches!(
        engine.attach_runner(
            claimed.run_spec.task_id,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            process_ref,
            10,
        ),
        Err(TaskEngineError::InvalidRunnerRef)
    ));
}

fn claimed_memory_engine() -> (
    PersistentTaskEngine<MemoryVaultBroker, MemoryAuditLedger>,
    ClaimedTask,
) {
    let vault_uuid = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault_uuid);
    let input = path("input.md");
    broker
        .mutate(Mutation::create(
            input.clone(),
            b"input".to_vec(),
            "claimed-fixture",
        ))
        .expect("create input");
    let db = ControlDb::open_in_memory(vault_uuid).expect("open control db");
    let mut engine = PersistentTaskEngine::new(db, broker, MemoryAuditLedger::new());
    let task_id = engine.create_task(spec(input), 1).expect("create task");
    engine
        .command(task_id, TaskCommand::Run, 2)
        .expect("queue task");
    let claimed = engine
        .claim(ClaimRequest {
            worker_id: "worker".to_owned(),
            now_ms: 3,
            lease_until_ms: 100,
        })
        .expect("claim")
        .expect("claimable task");
    (engine, claimed)
}

fn report_for(claimed: &ClaimedTask, content: &[u8]) -> RunnerReport {
    RunnerReport {
        task_id: claimed.run_spec.task_id,
        attempt_id: claimed.run_spec.attempt_id,
        fence: claimed.run_spec.fence,
        artifacts: vec![RunnerArtifact {
            name: path("result.md"),
            content: content.to_vec(),
        }],
    }
}

#[test]
fn stub_report_chain_commits_manifest_after_audit_and_enters_needs_review() {
    let vault_uuid = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault_uuid);
    let input_path = path("项目/Studiolo/输入.md");
    broker
        .mutate(Mutation::create(
            input_path.clone(),
            "你好，M0".as_bytes().to_vec(),
            "fixture-input",
        ))
        .expect("create input");
    let db = ControlDb::open_in_memory(vault_uuid).expect("open control db");
    let ledger = MemoryAuditLedger::new();
    let mut engine = PersistentTaskEngine::new(db, broker, ledger);
    accepts_object_safe_seam(&mut engine);

    let task_id = engine
        .create_task(spec(input_path), 1)
        .expect("create task");
    let queued = engine
        .command(task_id, TaskCommand::Run, 2)
        .expect("queue task");
    assert_eq!(queued.status, TaskStatus::Queued);

    let claimed = engine
        .claim(ClaimRequest {
            worker_id: "worker-1".to_owned(),
            now_ms: 3,
            lease_until_ms: 1_003,
        })
        .expect("claim succeeds")
        .expect("task is claimable");
    assert_eq!(claimed.run_spec.task_id, task_id);
    assert_eq!(claimed.run_spec.inputs.len(), 2);
    assert!(
        claimed
            .run_spec
            .inputs
            .iter()
            .any(|input| input.content == "你好，M0".as_bytes())
    );

    let runner = StubRunner::new();
    let started = runner.start(&claimed.run_spec).expect("start stub");
    engine
        .attach_runner(
            task_id,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            started.runner_ref,
            4,
        )
        .expect("attach runner");
    engine
        .heartbeat(
            task_id,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            1_500,
            5,
        )
        .expect("heartbeat");

    let RunnerObservation::Completed(report) =
        runner.observe(&started.handle).expect("observe stub")
    else {
        panic!("stub completes synchronously");
    };
    let result = engine
        .report("stub-report-1", report, 6)
        .expect("report commits");
    assert_eq!(result.status, TaskStatus::NeedsReview);
    assert!(!result.replayed);

    let task = engine
        .task(task_id)
        .expect("task query")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::NeedsReview);
    assert_eq!(task.active_attempt, None);
    assert_eq!(task.last_attempt, Some(claimed.run_spec.attempt_id));
    assert_eq!(
        task.input_snapshot.as_deref(),
        Some(claimed.snapshot_id.as_str())
    );
    assert_eq!(task.spec.role, "implementer");

    let manifest = engine
        .broker()
        .read(&result.receipt.manifest_path)
        .expect("manifest is the commit point");
    assert!(!manifest.content.is_empty());
    assert_eq!(result.receipt.artifacts.len(), 1);

    let audit = engine
        .audit_ledger()
        .query(&AuditFilter::default().with_task(task_id))
        .expect("query audit");
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[0].event().phase(), AuditPhase::Intent);
    assert_eq!(audit[1].event().phase(), AuditPhase::Outcome);
}

#[test]
fn task_spec_contains_only_human_authored_fields() {
    let encoded = serde_json::to_value(spec(path("input.md"))).expect("serialize task spec");
    let object = encoded.as_object().expect("task spec is an object");
    let mut keys: Vec<_> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "adapter",
            "allow_db",
            "allow_web",
            "input_paths",
            "instruction",
            "parent",
            "project",
            "retry_policy",
            "role",
            "timeout",
            "title",
        ]
    );
    assert!(!object.contains_key("status"));
    assert!(!object.contains_key("active_attempt"));
    assert!(!object.contains_key("last_attempt"));
    assert!(!object.contains_key("input_snapshot"));
}

#[test]
fn identical_report_replay_returns_the_same_manifest_without_duplicate_audit() {
    let (mut engine, claimed) = claimed_memory_engine();
    let report = report_for(&claimed, b"candidate");

    let first = engine
        .report("stable-report", report.clone(), 10)
        .expect("first report");
    let replay = engine
        .report("stable-report", report, 11)
        .expect("idempotent replay");

    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(replay.receipt, first.receipt);
    assert_eq!(
        engine
            .audit_ledger()
            .query(&AuditFilter::default())
            .expect("query audit")
            .len(),
        2
    );
}

#[test]
fn report_with_a_mismatched_fence_is_rejected_before_external_effects() {
    let (mut engine, claimed) = claimed_memory_engine();
    let mut report = report_for(&claimed, b"candidate");
    report.fence = Fence::new(claimed.run_spec.fence.get() + 1).expect("non-zero fence");

    assert!(matches!(
        engine.report("wrong-fence", report, 10),
        Err(TaskEngineError::StaleFence)
    ));
    assert_eq!(
        engine
            .audit_ledger()
            .query(&AuditFilter::default())
            .expect("query audit")
            .len(),
        0
    );
    assert_eq!(
        engine
            .task(claimed.run_spec.task_id)
            .expect("task query")
            .expect("task exists")
            .status,
        TaskStatus::Running
    );
}

#[test]
fn restart_reconcile_revokes_and_abandons_running_without_restarting_work() {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).expect("tempdir");
    let vault_root = temp.path().join("canonical");
    let data_root = temp.path().join("workbench-data");
    let database_path = data_root.join("control.sqlite3");
    let mut broker = FsVaultBroker::open(&vault_root, &data_root).expect("open broker");
    let vault_uuid = broker.vault_uuid();
    let input = path("input.md");
    broker
        .mutate(Mutation::create(
            input.clone(),
            b"restart input".to_vec(),
            "restart-input",
        ))
        .expect("create input");
    let db = ControlDb::open(&database_path, vault_uuid).expect("open control db");
    let mut first = PersistentTaskEngine::new(db, broker, MemoryAuditLedger::new());
    let task_id = first.create_task(spec(input), 1).expect("create task");
    first
        .command(task_id, TaskCommand::Run, 2)
        .expect("queue task");
    let claimed = first
        .claim(ClaimRequest {
            worker_id: "worker".to_owned(),
            now_ms: 3,
            lease_until_ms: 100,
        })
        .expect("claim")
        .expect("claimable task");
    assert_eq!(claimed.run_spec.task_id, task_id);
    drop(first);

    let reopened_db = ControlDb::open(&database_path, vault_uuid).expect("reopen control db");
    let reopened_broker = FsVaultBroker::open(&vault_root, &data_root).expect("reopen broker");
    let mut restarted =
        PersistentTaskEngine::new(reopened_db, reopened_broker, MemoryAuditLedger::new());
    let reconciled = restarted.reconcile(200).expect("reconcile restart");

    assert_eq!(reconciled.abandoned_attempts, 1);
    assert_eq!(reconciled.committed_attempts, 0);
    let task = restarted
        .task(task_id)
        .expect("task query")
        .expect("task exists");
    assert_eq!(task.status, TaskStatus::Abandoned);
    assert_eq!(task.active_attempt, None);
    assert_eq!(task.last_attempt, Some(claimed.run_spec.attempt_id));
}

#[test]
fn restart_rolls_manifest_forward_after_outcome_audit_recovers_without_duplicate_artifacts() {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).expect("tempdir");
    let vault_root = temp.path().join("canonical");
    let data_root = temp.path().join("workbench-data");
    let database_path = data_root.join("control.sqlite3");
    let mut broker = FsVaultBroker::open(&vault_root, &data_root).expect("open broker");
    let vault_uuid = broker.vault_uuid();
    let input = path("input.md");
    broker
        .mutate(Mutation::create(
            input.clone(),
            b"commit input".to_vec(),
            "commit-input",
        ))
        .expect("create input");
    let db = ControlDb::open(&database_path, vault_uuid).expect("open control db");
    let ledger = OutcomeSwitchLedger::new(true);
    let mut first = PersistentTaskEngine::new(db, broker, ledger.clone());
    let task_id = first.create_task(spec(input), 1).expect("create task");
    first
        .command(task_id, TaskCommand::Run, 2)
        .expect("queue task");
    let claimed = first
        .claim(ClaimRequest {
            worker_id: "worker".to_owned(),
            now_ms: 3,
            lease_until_ms: 100,
        })
        .expect("claim")
        .expect("claimable task");

    assert!(matches!(
        first.report("recoverable-report", report_for(&claimed, b"result"), 10),
        Err(TaskEngineError::AuditUnavailable)
    ));
    assert_eq!(
        first
            .task(task_id)
            .expect("task query")
            .expect("task exists")
            .status,
        TaskStatus::NeedsReview
    );
    let manifest_path = path(&format!(
        "sources/attempts/{}/{}/manifest.json",
        task_id, claimed.run_spec.attempt_id
    ));
    let artifact_path = path(&format!(
        "sources/attempts/{}/{}/result.md",
        task_id, claimed.run_spec.attempt_id
    ));
    let manifest_before = first
        .broker()
        .read(&manifest_path)
        .expect("manifest committed before outcome");
    let artifact_before = first
        .broker()
        .read(&artifact_path)
        .expect("artifact committed before manifest");
    drop(first);

    ledger.allow_outcome();
    let reopened_db = ControlDb::open(&database_path, vault_uuid).expect("reopen control db");
    let reopened_broker = FsVaultBroker::open(&vault_root, &data_root).expect("reopen broker");
    let mut restarted = PersistentTaskEngine::new(reopened_db, reopened_broker, ledger.clone());
    let reconciled = restarted.reconcile(20).expect("roll forward reservation");

    assert_eq!(reconciled.abandoned_attempts, 0);
    assert_eq!(reconciled.committed_attempts, 0);
    assert_eq!(reconciled.audit_outcomes, 1);
    assert_eq!(
        restarted
            .task(task_id)
            .expect("task query")
            .expect("task exists")
            .status,
        TaskStatus::NeedsReview
    );
    assert_eq!(
        restarted
            .broker()
            .read(&manifest_path)
            .expect("manifest remains")
            .revision,
        manifest_before.revision
    );
    assert_eq!(
        restarted
            .broker()
            .read(&artifact_path)
            .expect("artifact remains")
            .revision,
        artifact_before.revision
    );
    assert_eq!(
        ledger
            .query(&AuditFilter::default().with_task(task_id))
            .expect("query recovered audit")
            .len(),
        2
    );
    assert_eq!(
        std::fs::read_dir(vault_root.join(format!(
            "sources/attempts/{}/{}/",
            task_id, claimed.run_spec.attempt_id
        )))
        .expect("attempt directory")
        .count(),
        2
    );
    assert_eq!(ledger.outcome_appends.load(Ordering::SeqCst), 2);
    assert_eq!(restarted.reconcile(30).unwrap().audit_outcomes, 0);
    assert_eq!(
        ledger.outcome_appends.load(Ordering::SeqCst),
        2,
        "acknowledged outcomes are never scanned for replay"
    );
}

#[test]
fn restart_finishes_requested_cancellation_instead_of_overwriting_it_as_abandoned() {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let root = temp.path().join("vault");
    let data = temp.path().join("data");
    let db_path = data.join("control.sqlite3");
    let mut broker = FsVaultBroker::open(&root, &data).unwrap();
    let vault = broker.vault_uuid();
    broker
        .mutate(Mutation::create(
            path("input.md"),
            b"input".to_vec(),
            "fixture",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open(&db_path, vault).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let task = engine.create_task(spec(path("input.md")), 1).unwrap();
    engine.command(task, TaskCommand::Run, 2).unwrap();
    let claimed = engine
        .claim(ClaimRequest {
            worker_id: "worker".to_owned(),
            now_ms: 3,
            lease_until_ms: 100,
        })
        .unwrap()
        .unwrap();
    let runner = StubRunner::new();
    let started = runner.start(&claimed.run_spec).unwrap();
    engine
        .attach_runner(
            task,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            started.runner_ref,
            4,
        )
        .unwrap();
    assert_eq!(
        engine.command(task, TaskCommand::Cancel, 5).unwrap().status,
        TaskStatus::CancelRequested
    );
    drop(runner);
    drop(engine);

    let mut engine = PersistentTaskEngine::new(
        ControlDb::open(&db_path, vault).unwrap(),
        FsVaultBroker::open(&root, &data).unwrap(),
        MemoryAuditLedger::new(),
    );
    let summary = engine.reconcile(10).unwrap();
    assert_eq!(summary.abandoned_attempts, 0);
    assert_eq!(summary.cancelled_attempts, 1);
    let view = engine.task(task).unwrap().unwrap();
    assert_eq!(view.status, TaskStatus::Cancelled);
    assert_eq!(view.last_attempt, Some(claimed.run_spec.attempt_id));
    assert!(matches!(
        engine.report("late-after-restart", report_for(&claimed, b"late"), 11),
        Err(TaskEngineError::AlreadyTerminal)
    ));
    assert_eq!(engine.reconcile(12).unwrap().cancelled_attempts, 0);
}

struct OutcomeStateObserver {
    ledger: MemoryAuditLedger,
    database_path: std::path::PathBuf,
    vault: VaultUuid,
}

impl AuditLedger for OutcomeStateObserver {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.phase() == AuditPhase::Outcome {
            let task = event.subject().task_id().unwrap();
            let db = ControlDb::open(&self.database_path, self.vault).unwrap();
            assert_eq!(
                db.task(task).unwrap().unwrap().status,
                TaskStatus::NeedsReview,
                "outcome cannot claim a workflow state before T2 commits it"
            );
            assert_eq!(
                db.pending_terminal_audits().unwrap().len(),
                1,
                "T2 must reserve the audit obligation before append"
            );
        }
        self.ledger.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}

#[test]
fn audit_outcome_observes_the_already_committed_db_state_and_durable_obligation() {
    let temp = TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let db_path = temp.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault);
    broker
        .mutate(Mutation::create(
            path("input.md"),
            b"input".to_vec(),
            "fixture",
        ))
        .unwrap();
    let ledger = OutcomeStateObserver {
        ledger: MemoryAuditLedger::new(),
        database_path: db_path.clone(),
        vault,
    };
    let mut engine =
        PersistentTaskEngine::new(ControlDb::open(&db_path, vault).unwrap(), broker, ledger);
    let task = engine.create_task(spec(path("input.md")), 1).unwrap();
    engine.command(task, TaskCommand::Run, 2).unwrap();
    let claimed = engine
        .claim(ClaimRequest {
            worker_id: "worker".to_owned(),
            now_ms: 3,
            lease_until_ms: 100,
        })
        .unwrap()
        .unwrap();
    engine
        .report("truthful-outcome", report_for(&claimed, b"result"), 10)
        .unwrap();
    assert!(
        engine
            .control_db()
            .pending_terminal_audits()
            .unwrap()
            .is_empty()
    );
}

struct LostMutationAck {
    inner: MemoryVaultBroker,
    fail_next_ack: bool,
}

impl VaultReader for LostMutationAck {
    fn vault_uuid(&self) -> VaultUuid {
        self.inner.vault_uuid()
    }
    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        self.inner.read(path)
    }
}

impl VaultSnapshotter for LostMutationAck {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        self.inner.snapshot(inputs)
    }
}

impl SnapshotReader for LostMutationAck {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        self.inner.load_snapshot(id)
    }
}

impl VaultWriter for LostMutationAck {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError> {
        let receipt = self.inner.mutate(mutation)?;
        if std::mem::take(&mut self.fail_next_ack) {
            return Err(BrokerError::Io {
                operation: "injected lost projection acknowledgement",
                source: std::io::Error::other("interrupted after write"),
            });
        }
        Ok(receipt)
    }

    fn commit_attempt_artifacts(
        &mut self,
        commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        self.inner.commit_attempt_artifacts(commit)
    }
}

#[test]
fn historical_projection_receipt_cannot_acknowledge_an_externally_reverted_page() {
    let vault = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault);
    broker
        .mutate(Mutation::create(
            path("input.md"),
            b"input".to_vec(),
            "fixture",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    let task = engine.create_task(spec(path("input.md")), 1).unwrap();
    engine.command(task, TaskCommand::Run, 2).unwrap();
    let page = path(&format!("ops/tasks/{task}.md"));
    let original = engine.broker().read(&page).unwrap();
    let (db, broker, ledger) = engine.into_parts();
    let mut engine = PersistentTaskEngine::new(
        db,
        LostMutationAck {
            inner: broker,
            fail_next_ack: true,
        },
        ledger,
    );
    assert!(matches!(
        engine.project_pending(),
        Err(TaskEngineError::Broker(_))
    ));
    assert_eq!(engine.control_db().pending_outbox().unwrap().len(), 1);
    let (db, mut broker, ledger) = engine.into_parts();
    let projected = broker.read(&page).unwrap();
    assert_ne!(projected.content, original.content);
    broker
        .inner
        .mutate(Mutation::replace(
            page.clone(),
            original.content.clone(),
            projected.revision,
            "human-restored-original",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(db, broker, ledger);
    assert!(matches!(
        engine.project_pending(),
        Err(TaskEngineError::Projection(_))
    ));
    assert_eq!(
        engine.control_db().pending_outbox().unwrap().len(),
        1,
        "historical receipt must not consume the durable obligation"
    );
    assert_eq!(
        engine.broker().read(&page).unwrap().content,
        original.content
    );
}
