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

/// 夹具里的冻结预算，和 169 批准的一样：30 秒。
const FREEZE_MILLIS: u32 = 30_000;
/// 夹具里批准的间隔：一小时。
const INTERVAL_MILLIS: u64 = 3_600_000;

pub(super) struct Stage {
    pub(super) assembled: Assembled,
    pub(super) producer: Arc<BackupProducer>,
    pub(super) parent: std::path::PathBuf,
    pub(super) reader_gid: u32,
    retention_millis: u64,
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
    /// 演练「等得比冻结预算、比整次封存的总预算都久」：总预算取封存总预算和正文保留时长里较短的那个。
    pub(super) async fn with_budgets(retention_millis: u64, freeze_millis: u32) -> Self {
        Self::build(
            Assembled::new().await,
            true,
            retention_millis,
            freeze_millis,
        )
    }
    /// 主人登记过 TOTP、有一个接入方实例的六 owner；新根恢复的用例用它。
    pub(super) async fn with_owner_factor() -> Self {
        Self::assemble(Assembled::with_owner_factor().await, true, 600_000)
    }
    pub(super) fn assemble(assembled: Assembled, shared: bool, retention_millis: u64) -> Self {
        Self::build(assembled, shared, retention_millis, FREEZE_MILLIS)
    }
    fn build(
        assembled: Assembled,
        shared: bool,
        retention_millis: u64,
        freeze_millis: u32,
    ) -> Self {
        let parent = target(&assembled.fixture, "staging");
        if shared {
            fs::set_permissions(&parent, fs::Permissions::from_mode(0o750)).unwrap();
        }
        let reader_gid = fs::metadata(&parent).unwrap().gid();
        target(&assembled.fixture, "ephemeral");
        let producer = produce(
            &assembled,
            &parent,
            reader_gid,
            retention_millis,
            freeze_millis,
        );
        Self {
            assembled,
            producer,
            parent,
            reader_gid,
            retention_millis,
        }
    }
    /// 在给定时刻问一次「该不该封存」：生产者按这个时刻判断，账本照样记真实的落账时刻。
    pub(super) fn schedule_at(&self, now_ms: i64) -> QueueAnswer {
        self.producer.schedule_with(|| Ok(now_ms)).unwrap()
    }
    /// 第 `sequence` 号作业的原意图落账的时刻（Unix 毫秒）。
    pub(super) fn attempted_at(&self, sequence: u64) -> i64 {
        self.assembled
            .human
            .query(&AuditFilter::default().with_phase(AuditPhase::Intent))
            .unwrap()
            .iter()
            .find(|record| {
                studiolo_backup::worker::job::backup_event_key(record.event())
                    .is_some_and(|key| key.sequence.get() == sequence)
            })
            .expect("原意图已经落账")
            .recorded_at()
            .as_millisecond()
    }
    /// 人侧重启之后的生产者：同一本账本、同一组 owner、同一份固定配置，内存里的队列进度从零开始。
    pub(super) fn restarted(&self) -> Arc<BackupProducer> {
        produce(
            &self.assembled,
            &self.parent,
            self.reader_gid,
            self.retention_millis,
            FREEZE_MILLIS,
        )
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

fn produce(
    assembled: &Assembled,
    parent: &std::path::Path,
    reader_gid: u32,
    retention_millis: u64,
    freeze_millis: u32,
) -> Arc<BackupProducer> {
    let source_roots = [
        "vault", "data", "auth", "control", "agent", "policy", "audit",
    ]
    .map(|name| assembled.fixture.root.join(name))
    .to_vec();
    BackupProducer::new(
        assembled.human.clone(),
        Arc::new(assembled.inputs.clone()),
        ProducerPlan {
            // 对端核对只在真实连接上做；这里不走 socket。
            worker_uid: u32::MAX - 1,
            reader_gid,
            staging_parent: parent.to_owned(),
            ephemeral_root: assembled.fixture.root.join("ephemeral"),
            source_roots,
            min_interval_millis: INTERVAL_MILLIS,
            rpo_millis: 4_500_000,
            source_retention_millis: retention_millis,
            freeze_budget: FreezeBudget::new(freeze_millis).unwrap(),
            manifest: manifest(),
        },
    )
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

    // worker 写下结果之后不再是待办；离上一次尝试太近，不会马上再做一次。
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

/// 间隔从上一次尝试（原意图落账）那一刻数，不从封存做完（准入落账）重新数：准入总要晚上几分钟，
/// 从它数就会把下一次推到定时器的再下一格（BUG-057，169 上 10 分钟变成 15 分钟）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_next_seal_is_due_one_interval_after_the_last_attempt_not_after_its_admission() {
    let stage = Stage::new(true).await;
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    let attempted = stage.attempted_at(1);
    stage.producer.settle();
    stage.assembled.writable(true);
    let QueueAnswer::Pending(record) = stage.ask(QueueCommand::NextPending) else {
        panic!("封存完成后必须有待办");
    };
    // 封存做完才落准入，比原意图晚。
    assert!(record.recorded_at().as_millisecond() > attempted);
    let fact = AdmissionFact::from_receipt(&record).unwrap();
    let failure = BackupAuditRequest::append(
        fact.key().clone(),
        TARGET,
        backup_failure(fact.key(), FailureReason::UploadRejected).unwrap(),
    )
    .unwrap();
    stage.producer.dispatch(&failure.encode().unwrap()).unwrap();

    let minute = 60_000;
    let next = (attempted - attempted % minute) + i64::try_from(INTERVAL_MILLIS).unwrap();
    // 计划触发时刻的前一毫秒还没到，到了那个整分钟就开始，不管准入多晚落的账。
    assert_eq!(stage.schedule_at(next - 1), QueueAnswer::NotDue);
    stage.producer.settle();
    assert_eq!(stage.schedule_at(next), QueueAnswer::Started);
    stage.producer.settle();
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
    // 失败也占满批准的间隔（这里一小时）：和成功的同一条规则，到点之前不发新序号，到了才发。
    let attempted = stage.attempted_at(1);
    let next = (attempted - attempted % 60_000) + i64::try_from(INTERVAL_MILLIS).unwrap();
    assert_eq!(stage.schedule_at(next - 1), QueueAnswer::NotDue);
    stage.producer.settle();
    assert_eq!(stage.outcomes(2), 0, "不到批准的间隔不发新序号");
    assert_eq!(stage.schedule_at(next), QueueAnswer::Started);
    stage.producer.settle();
    assert_eq!(stage.outcomes(2), 1, "到点才重试");
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
