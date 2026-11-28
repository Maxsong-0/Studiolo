//! BUG-060：黑板审计的事件号就是请求摘要，账本按「事件号＋阶段」去重，同一个请求的 Intent、Outcome 各只有一条。
//!
//! - 暂时错误（控制库忙、Broker 读不到）没下结论：不能先记一条 rejected，否则同一请求重试成功时和它冲突，
//!   动作做了（号发出去了、任务建了）、调用方却拿到错误，之后同一请求永远失败。
//! - 确定拒绝照记 rejected；同一请求之后照样拒绝、什么都不做，条件变了也不会「先拒后成」把账本记成两种结论。
//!
//! 每个动作一条用例：只在一处改对、别处照旧，没改的那条会红。
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, MemoryAuditLedger,
};
use studiolo_control_db::{ControlDb, DbError, TaskStatus};
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_task_engine::*;
use studiolo_vault_broker::{BrokerError, MemoryVaultBroker, Mutation, VaultWriter};

const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
type Engine<L = MemoryAuditLedger> = PersistentTaskEngine<MemoryVaultBroker, L>;

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
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
            title: Some("BUG-060".into()),
            instruction: None,
        },
        acceptance_criteria: vec!["the contract passes".into()],
        files_owned: paths.iter().map(|p| path(p)).collect(),
        max_children: 8,
        artifact_target: None,
    }
}

fn handoff() -> Handoff {
    Handoff {
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

/// 控制库落在临时目录里的文件上，另一条连接才占得住它的写锁。
struct Fixture<L: AuditLedger = MemoryAuditLedger> {
    _temp: tempfile::TempDir,
    database: PathBuf,
    engine: Engine<L>,
}

fn fixture() -> Fixture {
    fixture_with(MemoryVaultBroker::with_uuid, |_| MemoryAuditLedger::new())
}

fn fixture_with<L: AuditLedger>(
    broker: impl FnOnce(VaultUuid) -> MemoryVaultBroker,
    ledger: impl FnOnce(&Path) -> L,
) -> Fixture<L> {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("control.sqlite3");
    let vault = VaultUuid::new();
    let engine = PersistentTaskEngine::new(
        ControlDb::open(&database, vault).unwrap(),
        broker(vault),
        ledger(&database),
    );
    Fixture {
        _temp: temp,
        database,
        engine,
    }
}

/// 另一条连接占住控制库的写锁：引擎的写事务等满忙等超时（5 秒）后报 SQLITE_BUSY，事务回滚、什么都没写。
fn writer_lock(database: &Path) -> rusqlite::Connection {
    let connection = rusqlite::Connection::open(database).unwrap();
    connection.execute_batch("BEGIN IMMEDIATE").unwrap();
    connection
}

#[track_caller]
fn assert_busy<T: std::fmt::Debug>(result: Result<T, TaskEngineError>) {
    match result {
        Err(TaskEngineError::ControlDb(DbError::Sqlite(error))) => assert_eq!(
            error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy),
            "{error}"
        ),
        other => panic!("应当是控制库忙，实际 {other:?}"),
    }
}

/// 某个动作的黑板请求审计（事件号 `board:` 开头），按落账顺序：Intent 记成 `intent`，Outcome 记成它的 status。
fn audited(ledger: &impl AuditLedger, action: &str) -> Vec<String> {
    ledger
        .query(&AuditFilter::default().with_action(action))
        .unwrap()
        .iter()
        .filter(|record| record.event().event_id().as_str().starts_with("board:"))
        .map(|record| match record.event().phase() {
            AuditPhase::Intent => "intent".to_owned(),
            AuditPhase::Outcome => record.event().payload().as_value()["status"]
                .as_str()
                .unwrap()
                .to_owned(),
        })
        .collect()
}

// ───────────── 暂时错误：不记结论，同一请求重试成功、账本只有一对 ─────────────

#[test]
fn a_number_issued_on_retry_after_a_busy_control_db_is_returned() {
    // BUG-060 原文的例子：以前第一次记 rejected，重试时号已经发出、Outcome 冲突，调用方拿到错误，之后永远失败。
    let mut f = fixture_with(
        |vault| {
            let mut broker = MemoryVaultBroker::with_uuid(vault);
            broker
                .mutate(Mutation::create(
                    path("notes/ADR-009-last.md"),
                    b"x".to_vec(),
                    "fixture",
                ))
                .unwrap();
            broker
        },
        |_| MemoryAuditLedger::new(),
    );
    let id = f.engine.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let lock = writer_lock(&f.database);
    assert_busy(f.engine.reserve_number("n1", id, &path("notes"), "ADR-"));
    drop(lock);
    assert_eq!(
        f.engine
            .reserve_number("n1", id, &path("notes"), "ADR-")
            .unwrap(),
        10,
        "重试拿到这一次发出的号"
    );
    assert_eq!(
        f.engine
            .reserve_number("n1", id, &path("notes"), "ADR-")
            .unwrap(),
        10,
        "之后再重放还是同一个号"
    );
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.number_reserved"),
        ["intent", "succeeded"]
    );
}

#[test]
fn a_task_created_on_retry_after_a_busy_control_db_is_returned() {
    let mut f = fixture();
    let lock = writer_lock(&f.database);
    assert_busy(f.engine.create_board_task("c1", spec(&["src"]), HASH, 0));
    drop(lock);
    let id = f
        .engine
        .create_board_task("c1", spec(&["src"]), HASH, 0)
        .unwrap();
    assert_eq!(
        f.engine
            .create_board_task("c1", spec(&["src"]), HASH, 0)
            .unwrap(),
        id
    );
    assert_eq!(f.engine.control_db().board_tasks().unwrap().len(), 1);
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.create"),
        ["intent", "succeeded"]
    );
}

#[test]
fn a_task_created_by_an_instance_on_retry_after_a_busy_control_db_is_returned() {
    let mut f = fixture();
    let lock = writer_lock(&f.database);
    assert_busy(
        f.engine
            .create_board_task_by_instance("c1", spec(&["src"]), HASH, "agent-a", 0),
    );
    drop(lock);
    let id = f
        .engine
        .create_board_task_by_instance("c1", spec(&["src"]), HASH, "agent-a", 0)
        .unwrap();
    assert_eq!(f.engine.control_db().board_tasks().unwrap().len(), 1);
    assert_eq!(
        f.engine
            .control_db()
            .board_task(id)
            .unwrap()
            .unwrap()
            .created_by_instance
            .as_deref(),
        Some("agent-a")
    );
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.create"),
        ["intent", "succeeded"]
    );
}

#[test]
fn a_claim_granted_on_retry_after_a_busy_control_db_is_returned() {
    let mut f = fixture();
    let id = f
        .engine
        .create_board_task("t", spec(&["src"]), HASH, 0)
        .unwrap();
    let lock = writer_lock(&f.database);
    assert_busy(f.engine.claim_board_task(id, "worker", 1_000, 1));
    drop(lock);
    let claim = f.engine.claim_board_task(id, "worker", 1_000, 1).unwrap();
    assert_eq!(claim.task.task.status, TaskStatus::Running);
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.claim"),
        ["intent", "succeeded"]
    );
}

#[test]
fn a_handoff_saved_on_retry_after_a_busy_control_db_is_accepted() {
    let mut f = fixture();
    let id = f
        .engine
        .create_board_task("t", spec(&["src"]), HASH, 0)
        .unwrap();
    let a = f
        .engine
        .claim_board_task(id, "first", 1_000, 1)
        .unwrap()
        .attempt;
    let lock = writer_lock(&f.database);
    assert_busy(
        f.engine
            .put_handoff(id, a.attempt_id, a.fence, handoff(), 2),
    );
    drop(lock);
    f.engine
        .put_handoff(id, a.attempt_id, a.fence, handoff(), 2)
        .unwrap();
    assert_eq!(
        f.engine.task(id).unwrap().unwrap().status,
        TaskStatus::Queued
    );
    assert_eq!(
        audited(f.engine.audit_ledger(), "handoff.created"),
        ["intent", "succeeded"]
    );
}

#[test]
fn a_receipt_accepted_on_retry_after_a_busy_control_db_is_returned() {
    let mut f = fixture();
    let id = f
        .engine
        .create_board_task("t", spec(&["src"]), HASH, 0)
        .unwrap();
    let a = f
        .engine
        .claim_board_task(id, "worker", 1_000, 1)
        .unwrap()
        .attempt;
    let lock = writer_lock(&f.database);
    assert_busy(
        f.engine
            .return_board_receipt(id, a.attempt_id, a.fence, receipt(), 2),
    );
    drop(lock);
    let view = f
        .engine
        .return_board_receipt(id, a.attempt_id, a.fence, receipt(), 2)
        .unwrap();
    assert_eq!(view.task.status, TaskStatus::Succeeded);
    assert_eq!(
        audited(f.engine.audit_ledger(), "handoff.returned"),
        ["intent", "succeeded"]
    );
}

#[test]
fn a_heartbeat_recorded_on_retry_after_a_busy_control_db_is_accepted() {
    let mut f = fixture();
    let id = f.engine.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let a = f
        .engine
        .claim_board_task(id, "worker", 1_000_000, 1)
        .unwrap()
        .attempt;
    let lock = writer_lock(&f.database);
    assert_busy(
        f.engine
            .heartbeat_progress(id, a.attempt_id, a.fence, 1_000_000, 1, 0, 2),
    );
    drop(lock);
    f.engine
        .heartbeat_progress(id, a.attempt_id, a.fence, 1_000_000, 1, 0, 2)
        .unwrap();
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.heartbeat"),
        ["intent", "succeeded"]
    );
}

#[test]
fn a_launch_failure_recorded_on_retry_after_a_busy_control_db_is_accepted() {
    let mut f = fixture();
    let id = f.engine.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let a = f
        .engine
        .claim_board_task(id, "worker", 10_000, 1)
        .unwrap()
        .attempt;
    let lock = writer_lock(&f.database);
    assert_busy(f.engine.launch_failed(id, a.attempt_id, a.fence, 2));
    drop(lock);
    assert!(
        f.engine
            .launch_failed(id, a.attempt_id, a.fence, 2)
            .unwrap(),
        "还能再试"
    );
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.launch_failed"),
        ["intent", "succeeded"]
    );
}

/// 记下某个动作的意图时顺手占住控制库写锁（只占一次）：忙出在意图之后、动作当中，测得到结果审计。
/// 对账的预演本身也要写锁，从外面先占住的话忙出在意图之前，走不到结果审计。
struct LockAfterIntent {
    inner: MemoryAuditLedger,
    database: PathBuf,
    action: &'static str,
    armed: AtomicBool,
    lock: Mutex<Option<rusqlite::Connection>>,
}

impl LockAfterIntent {
    fn new(database: &Path, action: &'static str) -> Self {
        Self {
            inner: MemoryAuditLedger::new(),
            database: database.to_owned(),
            action,
            armed: AtomicBool::new(true),
            lock: Mutex::new(None),
        }
    }

    fn release(&self) {
        self.lock.lock().unwrap().take();
    }
}

impl AuditLedger for LockAfterIntent {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let intent = event.phase() == AuditPhase::Intent && event.action() == self.action;
        let record = self.inner.append(event)?;
        if intent && self.armed.swap(false, Ordering::SeqCst) {
            *self.lock.lock().unwrap() = Some(writer_lock(&self.database));
        }
        Ok(record)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

#[test]
fn a_reconcile_round_retried_after_a_busy_control_db_applies_its_plan() {
    let mut f = fixture_with(MemoryVaultBroker::with_uuid, |database| {
        LockAfterIntent::new(database, "task.reconcile")
    });
    let id = f.engine.create_board_task("t", spec(&[]), HASH, 0).unwrap();
    let a = f
        .engine
        .claim_board_task(id, "worker", 1_000, 1)
        .unwrap()
        .attempt;
    assert_busy(f.engine.reconcile_periodic(2_000));
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.reconcile"),
        ["intent"],
        "忙出在执行计划时"
    );
    f.engine.audit_ledger().release();
    assert_eq!(
        f.engine.reconcile_periodic(2_000).unwrap().abandoned,
        vec![(id, a.attempt_id, "lease_expired".to_owned())]
    );
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.reconcile"),
        ["intent", "succeeded"]
    );
}

#[test]
fn a_claim_whose_input_appears_later_succeeds_on_retry() {
    // Broker 这一刻读不到（输入还没写进库）同样没下结论：输入就位后同一请求要能认领成功。
    let mut needs_input = spec(&[]);
    needs_input.task.input_paths = vec![path("inputs/brief.md")];
    let mut f = fixture();
    let id = f
        .engine
        .create_board_task("t", needs_input, HASH, 0)
        .unwrap();
    assert!(matches!(
        f.engine.claim_board_task(id, "worker", 1_000, 1),
        Err(TaskEngineError::Broker(BrokerError::NotFound(_)))
    ));
    let Fixture {
        _temp,
        database,
        engine,
    } = f;
    let (db, mut broker, ledger) = engine.into_parts();
    broker
        .mutate(Mutation::create(
            path("inputs/brief.md"),
            b"brief".to_vec(),
            "fixture",
        ))
        .unwrap();
    let mut f = Fixture {
        _temp,
        database,
        engine: Engine::new(db, broker, ledger),
    };
    f.engine.claim_board_task(id, "worker", 1_000, 1).unwrap();
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.claim"),
        ["intent", "succeeded"]
    );
}

/// Outcome 落账失败一次：动作已做、调用方拿到审计不可用；账本恢复后同一请求补上那条成功。
#[derive(Clone)]
struct OutcomeOutage {
    inner: Arc<MemoryAuditLedger>,
    down: Arc<AtomicBool>,
}

impl AuditLedger for OutcomeOutage {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.phase() == AuditPhase::Outcome && self.down.load(Ordering::SeqCst) {
            return Err(AuditError::Unavailable);
        }
        self.inner.append(event)
    }

    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.inner.query(filter)
    }
}

#[test]
fn an_outcome_lost_to_a_ledger_outage_is_written_by_the_retry() {
    let ledger = OutcomeOutage {
        inner: Arc::new(MemoryAuditLedger::new()),
        down: Arc::new(AtomicBool::new(true)),
    };
    let mut f = fixture_with(MemoryVaultBroker::with_uuid, |_| ledger.clone());
    assert!(matches!(
        f.engine.create_board_task("c1", spec(&[]), HASH, 0),
        Err(TaskEngineError::AuditUnavailable)
    ));
    assert_eq!(
        f.engine.control_db().board_tasks().unwrap().len(),
        1,
        "动作已做"
    );
    ledger.down.store(false, Ordering::SeqCst);
    let id = f
        .engine
        .create_board_task("c1", spec(&[]), HASH, 0)
        .unwrap();
    assert_eq!(f.engine.control_db().board_tasks().unwrap()[0].task_id, id);
    assert_eq!(
        audited(ledger.inner.as_ref(), "task.create"),
        ["intent", "succeeded"]
    );
}

// ───────────── 确定拒绝：记 rejected，同一请求之后照样拒绝 ─────────────

#[test]
fn a_refused_request_stays_refused_after_its_condition_clears() {
    // 父任务被认领时不能建子任务；父任务交接回到排队后，同一个请求不能「先拒后成」：
    // 以前会真的建出子任务、再撞上先记的 rejected，调用方拿到 IdempotencyMismatch。
    let mut f = fixture();
    let parent = f
        .engine
        .create_board_task("p", spec(&["src"]), HASH, 0)
        .unwrap();
    let a = f
        .engine
        .claim_board_task(parent, "parent", 1_000, 1)
        .unwrap()
        .attempt;
    let mut child = spec(&["src/a.rs"]);
    child.task.parent = Some(parent);
    assert!(matches!(
        f.engine.create_board_task("c", child.clone(), HASH, 2),
        Err(TaskEngineError::ControlDb(DbError::BoardRejected(_)))
    ));
    f.engine
        .put_handoff(parent, a.attempt_id, a.fence, handoff(), 3)
        .unwrap();
    assert_eq!(
        f.engine.task(parent).unwrap().unwrap().status,
        TaskStatus::Queued
    );
    let replay = f.engine.create_board_task("c", child.clone(), HASH, 4);
    assert!(
        matches!(replay, Err(TaskEngineError::RequestAlreadyRejected)),
        "同一请求照样拒绝，不是撞账本：{replay:?}"
    );
    assert_eq!(
        f.engine.control_db().board_tasks().unwrap().len(),
        1,
        "被拒过的请求重放时什么都不做"
    );
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.create"),
        ["intent", "succeeded", "intent", "rejected"],
        "账本里那个请求只有一个结论"
    );
    let fresh = f.engine.create_board_task("c-2", child, HASH, 5).unwrap();
    assert_eq!(
        f.engine.task(fresh).unwrap().unwrap().status,
        TaskStatus::Draft,
        "换一个新请求就能建"
    );
}

#[test]
fn a_claim_refused_for_held_paths_is_recorded_and_stays_refused() {
    let mut f = fixture();
    let first = f
        .engine
        .create_board_task("a", spec(&["src"]), HASH, 0)
        .unwrap();
    let second = f
        .engine
        .create_board_task("b", spec(&["src/x.rs"]), HASH, 0)
        .unwrap();
    let held = f
        .engine
        .claim_board_task(first, "one", 10_000, 1)
        .unwrap()
        .attempt;
    assert!(matches!(
        f.engine.claim_board_task(second, "two", 10_000, 2),
        Err(TaskEngineError::PathsHeld { .. })
    ));
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.claim"),
        ["intent", "succeeded", "intent", "rejected"]
    );
    // 交接放开路径以后，同一个请求也不能「先拒后成」。
    f.engine
        .put_handoff(first, held.attempt_id, held.fence, handoff(), 3)
        .unwrap();
    let replay = f.engine.claim_board_task(second, "two", 10_000, 2);
    assert!(
        matches!(replay, Err(TaskEngineError::RequestAlreadyRejected)),
        "路径放开以后同一请求也照样拒绝：{replay:?}"
    );
    assert_eq!(
        f.engine.task(second).unwrap().unwrap().status,
        TaskStatus::Queued,
        "被拒过的请求重放时什么都不做（第一次认领时输入已冻结、排进了队列，没被认领）"
    );
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.claim"),
        ["intent", "succeeded", "intent", "rejected"],
        "重放不多记"
    );
    f.engine.claim_board_task(second, "two", 10_000, 4).unwrap();
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.claim"),
        [
            "intent",
            "succeeded",
            "intent",
            "rejected",
            "intent",
            "succeeded"
        ],
        "换一个时刻就是新请求，路径放开了就能认领"
    );
}

#[test]
fn an_invalid_request_is_recorded_as_rejected_once() {
    let mut f = fixture();
    let mut invalid = spec(&[]);
    invalid.acceptance_criteria = vec!["same".into(), "same".into()];
    assert!(matches!(
        f.engine.create_board_task("bad", invalid.clone(), HASH, 0),
        Err(TaskEngineError::InvalidTaskSpec(_))
    ));
    assert!(matches!(
        f.engine.create_board_task("bad", invalid.clone(), HASH, 0),
        Err(TaskEngineError::RequestAlreadyRejected)
    ));
    assert!(f.engine.control_db().board_tasks().unwrap().is_empty());
    assert_eq!(
        audited(f.engine.audit_ledger(), "task.create"),
        ["intent", "rejected"]
    );
}
