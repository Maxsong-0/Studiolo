//! 端到端演练：生产者真实封存 → 队列协议 → worker 真实状态机和状态目录 → 结果经封闭协议回到人侧账本。
//!
//! 这里走的是「来源过期」那条出口，所以不需要 restic：上传、复验、最终清单那一段由备份包的契约用例
//! （替身执行器）和显式运行的官方 CLI 用例覆盖，不在这条里。两个系统账号和真实 socket 也不在这条里。
use super::{producer::Stage, *};
use crate::backup::producer::{BackupProducer, TARGET};
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditFilter, AuditRecord, OperationPermit};
use studiolo_backup::{
    store::storage::BackupStore,
    worker::{
        audit::{AdmissionFact, BackupAuditRequest, QueueAnswer, QueueCommand},
        final_manifest::FinalManifestReader,
        job::{
            Advance, BackupWorker, JobOutcome, JobPorts, PortUnavailable, Transfer, WorkerError,
        },
        restic::{CredentialHolder, RepositoryRef},
        state::FailureReason,
    },
};

/// worker 一侧的账本接口：每个调用都编码成带键的审计请求，交给生产者的分发入口。
/// 生产上中间隔着固定 socket 和对端账号核对；这里只去掉了那一层传输。
struct Wire {
    producer: Arc<BackupProducer>,
    key: JobKey,
}
impl Wire {
    fn exchange(&self, request: BackupAuditRequest) -> Result<Option<AuditRecord>, AuditError> {
        let bytes = request.encode().map_err(|_| AuditError::Serialization)?;
        let reply = self.producer.dispatch(&bytes)?;
        request
            .decode_reply(&reply)
            .map_err(|_| AuditError::CorruptStore)
    }
}
impl AuditLedger for Wire {
    fn enter_operation(&self) -> Result<OperationPermit, AuditError> {
        Err(AuditError::Unavailable)
    }
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let request = BackupAuditRequest::append(self.key.clone(), TARGET, event)
            .map_err(|_| AuditError::Serialization)?;
        self.exchange(request)?.ok_or(AuditError::CorruptStore)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        let phase = filter.phase().ok_or(AuditError::Serialization)?;
        Ok(self
            .exchange(BackupAuditRequest::read(self.key.clone(), TARGET, phase))?
            .into_iter()
            .collect())
    }
}

/// 这条演练里数据面永远装配不起来：走到要上传的分支就说明决策表走错了。
struct NoDataPlane<'a>(&'a AdmissionFact);
impl JobPorts for NoDataPlane<'_> {
    fn source_live(&self) -> bool {
        self.0.check_live().is_ok()
    }
    fn transfer(&self) -> Result<Box<dyn Transfer + '_>, PortUnavailable> {
        Err(PortUnavailable)
    }
    fn final_reader(&self) -> Result<Box<dyn FinalManifestReader + '_>, PortUnavailable> {
        Err(PortUnavailable)
    }
}

/// 固定父目录里还留着几份封存正文。
fn sealed_directories(parent: &std::path::Path) -> usize {
    fs::read_dir(parent)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("sealed-")
        })
        .count()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sealed_job_whose_source_expires_is_closed_by_the_worker_over_the_closed_protocol() {
    // 封存正文只留 6 秒：够真实六 owner 封存完、worker 走完第一步，也等得到它过期。
    let stage = Stage::with_retention(true, 6_000).await;
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    stage.assembled.writable(true);
    assert_eq!(sealed_directories(&stage.parent), 1);
    let QueueAnswer::Pending(record) = stage.ask(QueueCommand::NextPending) else {
        panic!("封存完成后必须有待办");
    };
    // 队列给的原键只是线索：worker 用带键的准入请求重新取得批准。
    let hint = AdmissionFact::from_receipt(&record).unwrap();
    let wire = Wire {
        producer: stage.producer.clone(),
        key: hint.key().clone(),
    };
    let admitted = wire
        .exchange(BackupAuditRequest::admission(hint.key().clone(), TARGET))
        .unwrap()
        .unwrap();
    let fact = AdmissionFact::from_receipt(&admitted).unwrap();
    assert_eq!(fact.key(), hint.key());
    assert_eq!(fact.source(), hint.source());

    // worker 一侧：真实状态目录（显式初始化）、真实状态机，账本只经封闭协议。
    let state = target(&stage.assembled.fixture, "worker-state");
    let store = BackupStore::open(&state, true).unwrap();
    let worker = BackupWorker::new(
        &wire,
        &store,
        RepositoryRef::parse("/srv/backup/repo").unwrap(),
        CredentialHolder::BackupWorker,
        TARGET,
    )
    .unwrap();
    let ports = NoDataPlane(&fact);

    // 来源还有效、数据面装配不起来：接收原键，然后这一轮什么都不做，账本里没有结果。
    assert!(ports.source_live());
    assert_eq!(worker.advance(fact.key(), &ports), Ok(Advance::Deferred));
    assert_eq!(stage.outcomes(1), 0);
    assert!(matches!(
        stage.ask(QueueCommand::NextPending),
        QueueAnswer::Pending(_)
    ));

    // 等封存正文过期（最多二十秒）。
    let waited = Instant::now();
    while ports.source_live() {
        assert!(waited.elapsed() < Duration::from_secs(20), "封存一直没过期");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // 过期之后的一步：记 source_expired 失败，结果经协议写进人侧账本。
    assert_eq!(
        worker.advance(fact.key(), &ports),
        Ok(Advance::Outcome(JobOutcome::Failed(
            FailureReason::SourceExpired
        )))
    );
    assert_eq!(stage.outcomes(1), 1);

    // 生产者一侧：这个原键收口了，不再是待办；离上一次准入还不到最小间隔，不会马上再封存一次。
    assert_eq!(stage.ask(QueueCommand::NextPending), QueueAnswer::Idle);
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::NotDue);
    // 这一次 `schedule` 不封存，但照样带起后台回收：过期的封存正文从固定父目录里清掉了。
    stage.producer.settle();
    assert_eq!(sealed_directories(&stage.parent), 0);
    stage.assembled.writable(true);
    // 本地已经是终态、生产者也不再列它：再对同一个原键走一步被拒，账本里不多一条结果。
    assert_eq!(
        worker.advance(fact.key(), &ports),
        Err(WorkerError::EvidenceMismatch)
    );
    assert_eq!(stage.outcomes(1), 1);
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_seal_is_reclaimed_without_schedule_and_without_a_new_freeze() {
    // 定时器停了、worker 也没来取：过期的封存正文由只回收的周期任务清掉，不冻结、不发新序号。
    // 保留 6 秒，理由同上一条：够真实六 owner 封存完并写下准入。
    let stage = Stage::with_retention(true, 6_000).await;
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    stage.producer.settle();
    stage.assembled.writable(true);
    assert_eq!(sealed_directories(&stage.parent), 1);
    let QueueAnswer::Pending(record) = stage.ask(QueueCommand::NextPending) else {
        panic!("封存完成后必须有待办");
    };
    let fact = AdmissionFact::from_receipt(&record).unwrap();

    // 还在保留期里：回收不动它。
    assert!(fact.check_live().is_ok());
    stage.producer.reclaim();
    stage.producer.settle();
    assert_eq!(sealed_directories(&stage.parent), 1);

    let waited = Instant::now();
    while fact.check_live().is_ok() {
        assert!(waited.elapsed() < Duration::from_secs(20), "封存一直没过期");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    stage.producer.reclaim();
    stage.producer.settle();
    assert_eq!(sealed_directories(&stage.parent), 0);
    // 只回收：工作台一直可写，账本里只有第一次的原意图。
    stage.assembled.writable(true);
    assert!(stage.assembled.human.enter_operation().is_ok());
    let intents = stage
        .assembled
        .human
        .query(&AuditFilter::default().with_phase(studiolo_audit_ledger::AuditPhase::Intent))
        .unwrap()
        .iter()
        .filter(|record| studiolo_backup::worker::job::backup_event_key(record.event()).is_some())
        .count();
    assert_eq!(intents, 1);
    stage.assembled.workbench.shutdown().await.unwrap();
}
