//! 真实 `Broker`/SQLite 下的候选恢复，以及独立的主人批准边界。
use sha2::{Digest, Sha256};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, MemoryAuditLedger};
use studiolo_control_db::{ControlDb, TaskStatus};
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef, TaskId, VaultPath, VaultUuid,
};
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
    grant_board(&mut engine, task, &claim);
    (engine, task, claim)
}

fn grant_board<B, L>(engine: &mut PersistentTaskEngine<B, L>, task: TaskId, claim: &BoardClaim)
where
    B: VaultBroker + ArtifactStager,
    L: AuditLedger,
{
    grant_until(engine, task, claim, 4_000_000_000);
}

/// `retain_until` 是原批准的到期时刻（Unix 秒）；Broker 按系统时钟判断它，不接受调用方自报的时间。
fn grant_until<B, L>(
    engine: &mut PersistentTaskEngine<B, L>,
    task: TaskId,
    claim: &BoardClaim,
    retain_until: i64,
) where
    B: VaultBroker + ArtifactStager,
    L: AuditLedger,
{
    engine
        .authorize_artifacts(
            task,
            claim.attempt.attempt_id,
            claim.attempt.fence,
            &claim.attempt.snapshot_id,
            &ArtifactProvenance::new(
                SourcePolicyRef::new("fixture-source", "a".repeat(64)).unwrap(),
                DataLabels::new(
                    Classification::Restricted,
                    BackupScope::Full,
                    DesktopSync::Deny,
                    ModelEgress::Deny,
                )
                .unwrap(),
                retain_until,
                "b".repeat(64),
            )
            .unwrap(),
            1,
        )
        .unwrap();
}

struct LostManifestAck {
    inner: FsVaultBroker,
    /// 清单已经写出、回执却没送回来。
    fail: bool,
    /// 预留之后、第一篇正文之前进程就没了：这一次提交什么都没写。
    before: bool,
    /// 「过期且没开始写」这件事 Broker 这会儿证实不了（只读检查出错）。
    unprovable: bool,
    /// 接下来这么多次丢暂存会失败（落定之后、丢暂存之前出错）。
    discard_faults: std::sync::atomic::AtomicU32,
    /// Broker 整体出了问题（存储写满之类）：设上之后每次提交都这样失败，清掉就恢复。
    wide_fault: std::sync::Mutex<Option<std::io::ErrorKind>>,
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
    fn staged_reference(&self, plan: &ArtifactCommit) -> Result<StagedArtifactCommit, BrokerError> {
        self.inner.staged_reference(plan)
    }
    fn discard_staged_artifact_commit(
        &self,
        pointer: &StagedArtifactCommit,
    ) -> Result<(), BrokerError> {
        use std::sync::atomic::Ordering;
        if self
            .discard_faults
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
        {
            return Err(BrokerError::Io {
                operation: "discard staged commit",
                source: std::io::Error::other("fixture fault"),
            });
        }
        self.inner.discard_staged_artifact_commit(pointer)
    }
    fn expired_before_start(&self, plan: &ArtifactCommit) -> Result<bool, BrokerError> {
        if self.unprovable {
            return Err(BrokerError::Io {
                operation: "expiry proof unavailable",
                source: std::io::Error::other("fixture fault"),
            });
        }
        self.inner.expired_before_start(plan)
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
        if let Some(kind) = *self.wide_fault.lock().unwrap() {
            return Err(BrokerError::Io {
                operation: "broker storage",
                source: std::io::Error::from(kind),
            });
        }
        if std::mem::take(&mut self.before) {
            return Err(BrokerError::Io {
                operation: "crash before the first body",
                source: std::io::Error::other("fixture crash"),
            });
        }
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
            before: false,
            unprovable: false,
            discard_faults: Default::default(),
            wide_fault: Default::default(),
        },
        MemoryAuditLedger::new(),
    );
    let task = engine.create_board_task("runner", spec(), HASH, 0).unwrap();
    let claim = engine
        .claim_board_task(task, "runner-instance", 1000, 1)
        .unwrap();
    grant_board(&mut engine, task, &claim);
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
    // Broker 私有数据根里现存的暂存正文份数。
    let staged = || {
        std::fs::read_dir(&data)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("artifact-candidate-")
            })
            .count()
    };
    assert_eq!(staged(), 0, "提交完成后暂存已清理");
    let replay = engine.stage_board_runner_candidates(report, 4).unwrap();
    assert_eq!(Some(replay), view.candidate_receipt);
    assert_eq!(staged(), 0, "已完成的精确重放不重建暂存");
}
/// 数写者数据目录里还有几份暂存正文。
fn staged_candidates(data: &std::path::Path) -> usize {
    std::fs::read_dir(data)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("artifact-candidate-")
        })
        .count()
}

/// 预留成功、第一篇正文之前「进程没了」，然后等到原批准过期。返回时 attempt 还停在「提交中」，
/// 暂存正文还在，Vault 里一个字节都没写。
fn expired_before_first_body(
    temp: &tempfile::TempDir,
    unprovable: bool,
    discard_faults: u32,
) -> (
    PersistentTaskEngine<LostManifestAck, MemoryAuditLedger>,
    TaskId,
    BoardClaim,
) {
    let broker =
        FsVaultBroker::open(&temp.path().join("vault"), &temp.path().join("broker")).unwrap();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open(temp.path().join("control.sqlite3"), broker.vault_uuid()).unwrap(),
        LostManifestAck {
            inner: broker,
            fail: false,
            before: true,
            unprovable,
            discard_faults: discard_faults.into(),
            wide_fault: Default::default(),
        },
        MemoryAuditLedger::new(),
    );
    let task = engine.create_board_task("runner", spec(), HASH, 0).unwrap();
    let claim = engine
        .claim_board_task(task, "runner-instance", 1000, 1)
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    // 原批准再过两到三秒到期：暂存和预留时还有效。
    let retain_until = i64::try_from(now.as_secs()).unwrap() + 3;
    grant_until(&mut engine, task, &claim, retain_until);
    assert!(matches!(
        engine.stage_board_runner_candidates(
            report(&claim, b"diff --git a/src/a b/src/a\n+safe\n"),
            2
        ),
        Err(TaskEngineError::CommitFailed(_))
    ));
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Committing
    );
    assert_eq!(staged_candidates(&temp.path().join("broker")), 1);
    let wait = std::time::Duration::from_secs(u64::try_from(retain_until).unwrap()).saturating_sub(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap(),
    ) + std::time::Duration::from_millis(200);
    std::thread::sleep(wait);
    (engine, task, claim)
}

/// 审计里每种动作各有几条。
fn actions(ledger: &MemoryAuditLedger, action: &str) -> usize {
    ledger
        .query(&AuditFilter::default())
        .unwrap()
        .iter()
        .filter(|record| record.event().action() == action)
        .count()
}

/// 预留之后、还没写出任何正文，原批准就过期了：对账不替主人丢，照旧停在「提交中」、保留暂存；
/// 只有主人调用丢弃入口才落成失败并丢掉暂存（2026-11-16 用户拍板：过期候选停在待审）。
#[test]
fn expired_unstarted_candidates_wait_for_the_owner_and_only_the_owner_discards_them() {
    let temp = tempfile::TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let data = temp.path().join("broker");
    let (mut engine, task, claim) = expired_before_first_body(&temp, false, 0);
    let staged = || staged_candidates(&data);
    engine.reconcile_periodic(10).unwrap();
    engine.reconcile_periodic(11).unwrap();
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Committing
    );
    assert_eq!(staged(), 1, "对账不能替主人丢暂存");
    assert_eq!(engine.control_db().committing_attempts().unwrap().len(), 1);
    assert!(
        engine.commit_awaiting_discard(task).unwrap(),
        "人侧要能看出：提交已过期、等主人决定"
    );

    assert_eq!(
        engine.discard_expired_commit(task, 12).unwrap(),
        TaskStatus::Failed
    );
    assert!(!engine.commit_awaiting_discard(task).unwrap());
    let view = engine.board_detail(task, HASH).unwrap().unwrap();
    assert_eq!(view.task.status, TaskStatus::Failed);
    assert!(view.task.active_attempt.is_none());
    assert_eq!(view.task.last_attempt, Some(claim.attempt.attempt_id));
    assert!(view.candidate_receipt.is_none());
    assert_eq!(staged(), 0, "主人丢掉之后暂存正文必须清掉");
    assert!(
        engine
            .control_db()
            .committing_attempts()
            .unwrap()
            .is_empty()
    );
    // 再对账不改结果；再丢一次（比如结果没送回来、主人又点了一次）给回同一个状态，不再要证实。
    engine.reconcile_periodic(13).unwrap();
    assert_eq!(
        engine.discard_expired_commit(task, 14).unwrap(),
        TaskStatus::Failed
    );
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Failed
    );
    let (_db, _broker, ledger) = engine.into_parts();
    assert_eq!(actions(&ledger, "task.commit_expired"), 1);
    assert_eq!(
        actions(&ledger, "task.candidate_recovery_pending"),
        1,
        "同一个 attempt 对账两轮也只记一次"
    );
}

/// 过期了，但 Broker 这会儿证实不了「一个字节都没写」：主人的丢弃也被拒绝，提交和暂存原样保留，
/// 对账照常做完（不因为一个证实不了的提交整轮出错，那会挡住别的 attempt）。
#[test]
fn owner_discard_is_refused_when_expiry_cannot_be_proven() {
    let temp = tempfile::TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let (mut engine, task, _claim) = expired_before_first_body(&temp, true, 0);
    engine.reconcile_periodic(10).unwrap();
    assert!(
        !engine.commit_awaiting_discard(task).unwrap(),
        "证实不了就不提示主人丢"
    );
    assert!(engine.discard_expired_commit(task, 11).is_err());
    engine.reconcile_periodic(12).unwrap();
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Committing
    );
    assert_eq!(
        staged_candidates(&temp.path().join("broker")),
        1,
        "证实不了就不能丢暂存"
    );
    assert_eq!(engine.control_db().committing_attempts().unwrap().len(), 1);
    let (_db, _broker, ledger) = engine.into_parts();
    assert_eq!(actions(&ledger, "task.commit_expired"), 0);
    assert_eq!(
        actions(&ledger, "task.candidate_recovery_pending"),
        1,
        "同一个 attempt 对账两轮也只记一次"
    );
}

/// Broker 整体出了问题（这里是存储写满、只读、没有权限）不是哪一份预留的事：周期对账整轮报错，常驻循环
/// 停下来让人看见，不把每个提交中的 attempt 记成各自待恢复（复审 A1-F3）。故障过去之后照常对账：过期的
/// 提交停在待恢复、等主人决定，待恢复审计只记一次。
#[test]
fn broker_wide_failure_fails_the_round_instead_of_parking_each_attempt() {
    let temp = tempfile::TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let (mut engine, task, _claim) = expired_before_first_body(&temp, false, 0);
    for kind in [
        std::io::ErrorKind::StorageFull,
        std::io::ErrorKind::ReadOnlyFilesystem,
        std::io::ErrorKind::PermissionDenied,
    ] {
        *engine.broker().wide_fault.lock().unwrap() = Some(kind);
        assert!(
            matches!(
                engine.reconcile_periodic(10),
                Err(TaskEngineError::CommitFailed(BrokerError::Io { .. }))
            ),
            "{kind:?} 要让整轮报错"
        );
        assert_eq!(
            engine.task(task).unwrap().unwrap().status,
            TaskStatus::Committing
        );
    }
    *engine.broker().wide_fault.lock().unwrap() = None;
    engine.reconcile_periodic(11).unwrap();
    engine.reconcile_periodic(12).unwrap();
    assert!(engine.commit_awaiting_discard(task).unwrap());
    assert_eq!(
        staged_candidates(&temp.path().join("broker")),
        1,
        "整轮报错不能丢暂存"
    );
    let (_db, _broker, ledger) = engine.into_parts();
    assert_eq!(
        actions(&ledger, "task.candidate_recovery_pending"),
        1,
        "故障那几轮不记待恢复，恢复后同一个 attempt 只记一次"
    );
}

/// 落定已经提交、丢暂存那一步出错：主人再点一次就收拾干净，审计里仍然只有一条「提交过期」。
#[test]
fn owner_discard_that_fails_after_settling_can_be_retried() {
    let temp = tempfile::TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let data = temp.path().join("broker");
    let (mut engine, task, _claim) = expired_before_first_body(&temp, false, 1);
    engine.reconcile_periodic(10).unwrap();
    assert!(engine.discard_expired_commit(task, 11).is_err());
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Failed
    );
    assert_eq!(staged_candidates(&data), 1);
    assert_eq!(
        engine.discard_expired_commit(task, 12).unwrap(),
        TaskStatus::Failed
    );
    assert_eq!(staged_candidates(&data), 0);
    let (_db, _broker, ledger) = engine.into_parts();
    assert_eq!(actions(&ledger, "task.commit_expired"), 1);
}

/// 不是黑板的任务（这里是 stub 执行体）：终态提交同样带原批准、同样会过期，也只能由主人丢。
#[test]
fn expired_plain_task_commit_is_discarded_by_the_owner_too() {
    let temp = tempfile::TempDir::new_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let data = temp.path().join("broker");
    let broker = FsVaultBroker::open(&temp.path().join("vault"), &data).unwrap();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open(temp.path().join("control.sqlite3"), broker.vault_uuid()).unwrap(),
        LostManifestAck {
            inner: broker,
            fail: false,
            before: true,
            unprovable: false,
            discard_faults: Default::default(),
            wide_fault: Default::default(),
        },
        MemoryAuditLedger::new(),
    );
    let task = engine
        .create_task(
            TaskSpec {
                adapter: "stub".into(),
                ..spec().task
            },
            1,
        )
        .unwrap();
    assert!(
        !engine.commit_awaiting_discard(task).unwrap(),
        "草稿没有提交"
    );
    engine.command(task, TaskCommand::Run, 2).unwrap();
    let claimed = engine
        .claim(ClaimRequest {
            worker_id: "worker".into(),
            now_ms: 3,
            lease_until_ms: 100,
        })
        .unwrap()
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let retain_until = i64::try_from(now.as_secs()).unwrap() + 3;
    engine
        .authorize_artifacts(
            task,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            &claimed.snapshot_id,
            &ArtifactProvenance::new(
                SourcePolicyRef::new("fixture-source", "a".repeat(64)).unwrap(),
                DataLabels::new(
                    Classification::Restricted,
                    BackupScope::Full,
                    DesktopSync::Deny,
                    ModelEgress::Deny,
                )
                .unwrap(),
                retain_until,
                "b".repeat(64),
            )
            .unwrap(),
            4,
        )
        .unwrap();
    let report = RunnerReport {
        task_id: task,
        attempt_id: claimed.run_spec.attempt_id,
        fence: claimed.run_spec.fence,
        artifacts: vec![RunnerArtifact {
            name: path("result.md"),
            content: b"body".to_vec(),
        }],
    };
    assert!(matches!(
        engine.report("plain-report", report, 5),
        Err(TaskEngineError::CommitFailed(_))
    ));
    assert_eq!(staged_candidates(&data), 1);
    assert!(
        !engine.commit_awaiting_discard(task).unwrap(),
        "原批准还没过期：不提示主人丢"
    );
    let wait = std::time::Duration::from_secs(u64::try_from(retain_until).unwrap()).saturating_sub(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap(),
    ) + std::time::Duration::from_millis(200);
    std::thread::sleep(wait);

    engine.reconcile_periodic(10).unwrap();
    assert_eq!(
        engine.task(task).unwrap().unwrap().status,
        TaskStatus::Committing
    );
    assert_eq!(staged_candidates(&data), 1, "对账不能替主人丢暂存");
    assert!(engine.commit_awaiting_discard(task).unwrap());
    assert_eq!(
        engine.discard_expired_commit(task, 11).unwrap(),
        TaskStatus::Failed
    );
    assert!(!engine.commit_awaiting_discard(task).unwrap());
    assert_eq!(staged_candidates(&data), 0);
    assert!(
        engine
            .control_db()
            .committing_attempts()
            .unwrap()
            .is_empty()
    );
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
