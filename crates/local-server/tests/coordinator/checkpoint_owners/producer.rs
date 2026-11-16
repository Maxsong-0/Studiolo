//! 生产者接真实六 owner：收到 `schedule` 才封存，封存完的原键才成为 worker 的待办。
use super::{fixture::Assembled, seal::manifest, *};
use crate::backup::producer::{BackupProducer, ProducerPlan, TARGET};
use std::os::unix::fs::MetadataExt as _;
use studiolo_audit_ledger::{AuditFilter, AuditPhase};
use studiolo_backup::{
    checkpoint::staging::SealedCheckpoint,
    worker::{
        audit::{AdmissionFact, BackupAuditRequest, BackupQueueRequest, QueueAnswer, QueueCommand},
        job::backup_failure,
        state::FailureReason,
    },
};
use studiolo_core::checkpoint::InstantMillis;

pub(super) struct Stage {
    pub(super) assembled: Assembled,
    pub(super) producer: Arc<BackupProducer>,
    pub(super) parent: std::path::PathBuf,
    pub(super) reader_gid: u32,
}
impl Stage {
    /// `shared` 为 false 时封存父目录保持 0700：读取组条件不满足，封存一定做不成。
    pub(super) async fn new(shared: bool) -> Self {
        Self::with_retention(shared, 600_000).await
    }
    /// `retention_millis` 是封存正文留多久；演练「来源过期」时给一个很短的值。
    pub(super) async fn with_retention(shared: bool, retention_millis: u64) -> Self {
        Self::assemble(Assembled::new().await, shared, retention_millis)
    }
    /// 主人登记过 TOTP、有一个接入方实例的六 owner；新根恢复的用例用它。
    pub(super) async fn with_owner_factor() -> Self {
        Self::assemble(Assembled::with_owner_factor().await, true, 600_000)
    }
    fn assemble(assembled: Assembled, shared: bool, retention_millis: u64) -> Self {
        let parent = target(&assembled.fixture, "staging");
        if shared {
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o750)).unwrap();
        }
        let reader_gid = fs::metadata(&parent).unwrap().gid();
        let ephemeral = target(&assembled.fixture, "ephemeral");
        let source_roots = [
            "vault", "data", "auth", "control", "agent", "policy", "audit",
        ]
        .map(|name| assembled.fixture.root.join(name))
        .to_vec();
        let producer = BackupProducer::new(
            assembled.human.clone(),
            Arc::new(assembled.inputs.clone()),
            ProducerPlan {
                // 对端核对只在真实连接上做；这里不走 socket。
                worker_uid: u32::MAX - 1,
                reader_gid,
                staging_parent: parent.clone(),
                ephemeral_root: ephemeral,
                source_roots,
                min_interval_millis: 3_600_000,
                source_retention_millis: retention_millis,
                freeze_budget: FreezeBudget::new(30_000).unwrap(),
                manifest: manifest(),
            },
        );
        Self {
            assembled,
            producer,
            parent,
            reader_gid,
        }
    }
    pub(super) fn ask(&self, command: QueueCommand) -> QueueAnswer {
        let request = BackupQueueRequest::new(TARGET, command);
        request
            .decode_reply(&self.producer.dispatch(&request.encode().unwrap()).unwrap())
            .unwrap()
    }
    pub(super) fn outcomes(&self, sequence: u64) -> usize {
        self.assembled
            .human
            .query(&AuditFilter::default().with_phase(AuditPhase::Outcome))
            .unwrap()
            .iter()
            .filter(|record| {
                studiolo_backup::worker::job::backup_event_key(record.event())
                    .is_some_and(|key| key.sequence.get() == sequence)
            })
            .count()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn producer_seals_only_when_scheduled_and_hands_the_admitted_key_to_the_worker() {
    let stage = Stage::new(true).await;
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    // 封存还在做或者刚做完，都还有没收口的作业：再催一次不会另起一次冻结。
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Outstanding);
    stage.producer.settle();
    stage.assembled.writable(true);

    let QueueAnswer::Pending(record) = stage.ask(QueueCommand::NextPending) else {
        panic!("封存完成后必须有待办");
    };
    let fact = AdmissionFact::from_receipt(&record).unwrap();
    assert_eq!(fact.key().sequence.get(), 1);
    assert_eq!(fact.target(), TARGET);
    assert!(fact.check_live().is_ok());
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    // 准入指向的封存真的在固定父目录里，读取组一侧能按原引用重开。
    let sealed = SealedCheckpoint::open_shared(
        &stage.parent,
        fact.source(),
        stage.reader_gid,
        InstantMillis::from_millis(now),
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    assert!(sealed.manifest().audit_coverage.is_exact());

    // 带键的请求走原准入端口：读得到原意图；另一个目标类别的同键请求被拒。
    let read = BackupAuditRequest::read(fact.key().clone(), TARGET, AuditPhase::Intent);
    assert!(
        read.decode_reply(&stage.producer.dispatch(&read.encode().unwrap()).unwrap())
            .unwrap()
            .is_some()
    );
    let foreign = BackupAuditRequest::read(
        fact.key().clone(),
        BackupTargetKind::LocalTemporaryRepository,
        AuditPhase::Intent,
    );
    assert!(stage.producer.dispatch(&foreign.encode().unwrap()).is_err());
    let unknown = JobKey {
        sequence: NonZeroU64::new(2).unwrap(),
        checkpoint_id: CheckpointId::parse("never-admitted").unwrap(),
    };
    let unadmitted = BackupAuditRequest::read(unknown, TARGET, AuditPhase::Intent);
    assert!(
        stage
            .producer
            .dispatch(&unadmitted.encode().unwrap())
            .is_err(),
        "没有准入的键不能借这个端口读写账本"
    );

    // worker 写下结果之后不再是待办；离上一次封存太近，不会马上再做一次。
    let failure = BackupAuditRequest::append(
        fact.key().clone(),
        TARGET,
        backup_failure(fact.key(), FailureReason::UploadRejected).unwrap(),
    )
    .unwrap();
    stage.producer.dispatch(&failure.encode().unwrap()).unwrap();
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::NotDue);
    stage.producer.settle();
    assert_eq!(stage.outcomes(1), 1);
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_seal_that_cannot_be_made_is_recorded_as_failed_and_not_retried_before_the_interval() {
    let stage = Stage::new(false).await;
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    // 没做成：屏障全部放开，原键有一条失败结果，worker 没有待办。
    stage.assembled.writable(true);
    assert!(stage.assembled.human.enter_operation().is_ok());
    assert!(stage.assembled.agent.enter_operation().is_ok());
    assert_eq!(stage.outcomes(1), 1);
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::NotDue);
    stage.producer.settle();
    // 失败也不缩短批准的间隔（这里一小时）：到点之前不发新序号，间隔的边界由单元测试钉住。
    assert_eq!(stage.outcomes(2), 0, "不到批准的间隔不发新序号");
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_intent_left_without_admission_or_outcome_is_closed_before_anything_new() {
    let stage = Stage::new(true).await;
    // 上一次进程在写了原意图之后、封存做完之前没了：账本里只剩一条孤零零的意图。
    let orphan = JobKey {
        sequence: NonZeroU64::new(7).unwrap(),
        checkpoint_id: CheckpointId::parse("left-behind").unwrap(),
    };
    stage
        .assembled
        .human
        .append(backup_intent(&orphan, TARGET).unwrap())
        .unwrap();
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::NotDue);
    stage.producer.settle();
    assert_eq!(stage.outcomes(7), 1, "孤儿意图先补失败结果");
    assert_eq!(stage.outcomes(8), 0, "刚有过一次尝试，不马上发新序号");
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    stage.producer.settle();
    stage.assembled.workbench.shutdown().await.unwrap();
}
