//! 投影幂等键的判别位（T-052）：Broker 把每把幂等键钉死在一次请求的终态上，
//! 因此键必须同时做到「同一请求的重试落在同一把键上」与「任何不同的写入落在不同的键上」。
//!
//! 这三条契约分别覆盖恢复（被钉死的拒绝不能让投影永久搁浅）、行为（重放不换键）
//! 与失败（同键异请求必须被明确拒绝，且只隔离它自己的任务）。

use std::collections::BTreeSet;

use studiolo_audit_ledger::MemoryAuditLedger;
use studiolo_control_db::{ControlDb, TaskStatus};
use studiolo_core::{Revision, VaultPath, VaultUuid};
use studiolo_task_engine::{
    PersistentTaskEngine, RetryPolicy, TaskEngine, TaskEngineError, TaskSpec,
};
use studiolo_vault_broker::{
    ArtifactCommit, ArtifactCommitReceipt, BrokerError, MemoryVaultBroker, Mutation,
    MutationReceipt, Precondition, Snapshot, SnapshotId, SnapshotInput, SnapshotReader,
    VaultObject, VaultReader, VaultSnapshotter, VaultWriter,
};

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
        title: Some("投影幂等".to_owned()),
        instruction: Some("总结输入并生成结果".to_owned()),
    }
}

/// 复刻 Broker 的按键终态：一把键上的 CAS 拒绝永久保持拒绝，后续同键请求只拿到重放。
///
/// `pending_rejections` 模拟「任务页在 read 与 mutate 之间被别人建出来、随后又消失」：
/// 头几次 `MustNotExist` 写入撞上占位者而被拒绝并被钉死，之后页面又不存在了。
struct SealingBroker {
    inner: MemoryVaultBroker,
    /// 每一次写入用过的幂等键，按序记录。
    keys: Vec<String>,
    /// 还要把多少次 `MustNotExist` 写入钉成拒绝终态。
    pending_rejections: usize,
    /// 拒绝时回报的占位者 revision。
    occupant: Revision,
    /// 已被钉死的键。
    sealed: BTreeSet<String>,
}

impl SealingBroker {
    fn new(inner: MemoryVaultBroker, occupant: Revision, pending_rejections: usize) -> Self {
        Self {
            inner,
            keys: Vec::new(),
            pending_rejections,
            occupant,
            sealed: BTreeSet::new(),
        }
    }

    fn rejection(&self, path: VaultPath) -> BrokerError {
        BrokerError::RevisionConflict {
            path,
            expected: "absence".to_owned(),
            actual: Some(self.occupant),
        }
    }
}

impl VaultReader for SealingBroker {
    fn vault_uuid(&self) -> VaultUuid {
        self.inner.vault_uuid()
    }
    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        self.inner.read(path)
    }
}

impl VaultSnapshotter for SealingBroker {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        self.inner.snapshot(inputs)
    }
}

impl SnapshotReader for SealingBroker {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        self.inner.load_snapshot(id)
    }
}

impl VaultWriter for SealingBroker {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError> {
        self.keys.push(mutation.idempotency_key.clone());
        if self.sealed.contains(&mutation.idempotency_key) {
            // 同键重放：Broker 只会把当初的拒绝原样还给调用方。
            return Err(self.rejection(mutation.path));
        }
        if mutation.precondition == Precondition::MustNotExist && self.pending_rejections > 0 {
            self.pending_rejections -= 1;
            self.sealed.insert(mutation.idempotency_key.clone());
            return Err(self.rejection(mutation.path));
        }
        self.inner.mutate(mutation)
    }

    fn commit_attempt_artifacts(
        &mut self,
        commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        self.inner.commit_attempt_artifacts(commit)
    }
}

fn sealing_engine(
    pending_rejections: usize,
) -> PersistentTaskEngine<SealingBroker, MemoryAuditLedger> {
    let vault = VaultUuid::new();
    let mut inner = MemoryVaultBroker::with_uuid(vault);
    inner
        .mutate(Mutation::create(
            path("input.md"),
            b"input".to_vec(),
            "fixture",
        ))
        .expect("fixture input is writable");
    let occupant = Revision::compute(vault, &path("input.md"), b"occupant");
    PersistentTaskEngine::new(
        ControlDb::open_in_memory(vault).expect("in-memory control db opens"),
        SealingBroker::new(inner, occupant, pending_rejections),
        MemoryAuditLedger::new(),
    )
}

#[test]
fn a_sealed_create_rejection_does_not_strand_the_projection_once_the_page_is_absent_again() {
    let mut engine = sealing_engine(1);
    let task = engine
        .create_task(spec(path("input.md")), 1)
        .expect("draft is created");
    let page = path(&format!("ops/tasks/{task}.md"));

    // 第一次写入撞上占位者并被钉成拒绝终态；页面随后又不存在，引擎必须换一把没被封死的键。
    assert_eq!(
        engine.project_pending().expect("projection recovers"),
        1,
        "被钉死的旧键不能让这条投影永久搁浅"
    );

    let keys = &engine.broker().keys;
    assert_eq!(keys.len(), 2, "只用了两次写入：被钉死的一把加换出来的一把");
    assert_ne!(
        keys[0], keys[1],
        "前提当下仍然成立说明拒绝只是重放，下一次写入必须换键：{keys:?}"
    );
    assert!(
        keys.iter().all(|key| key.contains(&task.to_string())),
        "两把键都绑定本任务身份：{keys:?}"
    );
    assert!(engine.broker().read(&page).is_ok(), "任务页最终被建出来");
    assert!(
        engine
            .control_db()
            .pending_outbox()
            .expect("outbox readable")
            .is_empty(),
        "投影成功后才消掉持久义务"
    );
}

#[test]
fn replaying_the_same_projection_request_reuses_the_same_idempotency_keys() {
    // 三次写入全部被钉死：本轮用尽尝试次数后隔离该任务。
    let mut engine = sealing_engine(3);
    let task = engine
        .create_task(spec(path("input.md")), 1)
        .expect("draft is created");

    for pass in 0..2 {
        assert!(
            matches!(
                engine.project_pending(),
                Err(TaskEngineError::ProjectionIncomplete { delivered: 0, ref failures })
                    if failures.len() == 1 && failures[0].task_id == task
            ),
            "第 {pass} 轮应当只隔离这条任务并保留待办"
        );
    }

    let keys = &engine.broker().keys;
    assert_eq!(keys.len(), 6, "两轮各三次写入");
    let (first_pass, second_pass) = keys.split_at(3);
    assert_eq!(
        first_pass, second_pass,
        "崩溃重启后重放同一条 outbox 事件必须落在同一批键上，不能被当成新请求"
    );
    assert_eq!(
        first_pass.iter().collect::<BTreeSet<_>>().len(),
        3,
        "一轮之内每推进一次判别位就必须换一把 Broker 没封死的键：{first_pass:?}"
    );
    assert_eq!(
        engine
            .control_db()
            .pending_outbox()
            .expect("outbox readable")
            .len(),
        1,
        "隔离不吞掉持久义务"
    );
}

/// 在引擎用到某把键之前，先用**另一个请求**（不同路径、不同正文）占住同一把键。
struct KeySquattingBroker {
    inner: MemoryVaultBroker,
    /// 仍要抢注的任务页；一旦抢注过就放行。
    squat_for: Option<VaultPath>,
    /// 抢注用的无关路径。
    decoy: VaultPath,
}

impl VaultReader for KeySquattingBroker {
    fn vault_uuid(&self) -> VaultUuid {
        self.inner.vault_uuid()
    }
    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError> {
        self.inner.read(path)
    }
}

impl VaultSnapshotter for KeySquattingBroker {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError> {
        self.inner.snapshot(inputs)
    }
}

impl SnapshotReader for KeySquattingBroker {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError> {
        self.inner.load_snapshot(id)
    }
}

impl VaultWriter for KeySquattingBroker {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError> {
        if self.squat_for.as_ref() == Some(&mutation.path) {
            self.squat_for = None;
            // 同一把幂等键先被一个完全不同的请求占住。
            self.inner.mutate(Mutation::create(
                self.decoy.clone(),
                b"another request".to_vec(),
                mutation.idempotency_key.clone(),
            ))?;
        }
        self.inner.mutate(mutation)
    }

    fn commit_attempt_artifacts(
        &mut self,
        commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError> {
        self.inner.commit_attempt_artifacts(commit)
    }
}

#[test]
fn a_key_already_bound_to_a_different_request_is_refused_and_quarantines_only_its_own_task() {
    let vault = VaultUuid::new();
    let mut inner = MemoryVaultBroker::with_uuid(vault);
    inner
        .mutate(Mutation::create(
            path("input.md"),
            b"input".to_vec(),
            "fixture",
        ))
        .expect("fixture input is writable");
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(vault).expect("in-memory control db opens"),
        inner,
        MemoryAuditLedger::new(),
    );
    // 序号更小的受害任务排在队头，健康任务的投影排在它后面。
    let squatted = engine
        .create_task(spec(path("input.md")), 1)
        .expect("draft is created");
    let healthy = engine
        .create_task(spec(path("input.md")), 2)
        .expect("draft is created");
    let squatted_page = path(&format!("ops/tasks/{squatted}.md"));
    let healthy_page = path(&format!("ops/tasks/{healthy}.md"));

    let (db, inner, ledger) = engine.into_parts();
    let mut engine = PersistentTaskEngine::new(
        db,
        KeySquattingBroker {
            inner,
            squat_for: Some(squatted_page.clone()),
            decoy: path("notes/unrelated.md"),
        },
        ledger,
    );

    match engine.project_pending() {
        Err(TaskEngineError::ProjectionIncomplete {
            delivered,
            failures,
        }) => {
            assert_eq!(delivered, 1, "健康任务的投影不被队头的键冲突挡住");
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0].task_id, squatted);
            assert!(
                failures[0].reason.contains("idempotency key"),
                "拒绝原因必须点名幂等键冲突而不是含糊的页面变化：{}",
                failures[0].reason
            );
        }
        other => panic!("expected ProjectionIncomplete, got {other:?}"),
    }

    assert!(
        matches!(
            engine.broker().read(&squatted_page),
            Err(BrokerError::NotFound(_))
        ),
        "被拒绝的请求不能静默继承另一个请求的终态而把页面当成已写出"
    );
    assert!(engine.broker().read(&healthy_page).is_ok());
    let pending = engine
        .control_db()
        .pending_outbox()
        .expect("outbox readable");
    assert_eq!(
        pending
            .iter()
            .map(|event| event.task_id)
            .collect::<Vec<_>>(),
        vec![squatted],
        "受害任务的持久义务保留待办，健康任务已交付"
    );
}

#[test]
fn projected_status_survives_the_recovered_key() {
    let mut engine = sealing_engine(1);
    let task = engine
        .create_task(spec(path("input.md")), 1)
        .expect("draft is created");
    engine.project_pending().expect("projection recovers");
    assert_eq!(
        engine
            .task(task)
            .expect("task readable")
            .expect("task exists")
            .status,
        TaskStatus::Draft,
        "换键只影响写入身份，不改变被投影的事实"
    );
}
