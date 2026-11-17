//! 生产者接真实账本派仓库抽查、收结果：只按账本定组号，结果要有同一检查键的原意图才落账。
use super::{producer::Stage, *};
use crate::backup::{BackupJobsProbe, CheckVerdict, producer::TARGET};
use studiolo_audit_ledger::{AuditError, AuditFilter, AuditPhase, AuditRecord};
use studiolo_backup::worker::{
    audit::{QueueAnswer, QueueCommand, RepositoryCheckReport},
    repository_check::{
        CheckAssignment, CheckBucket, CheckEvent, CheckKey, CheckOutcome, CheckPurpose,
        CheckRecorder, IncompleteReason, RepositoryDigest, read_check_event,
    },
    restic::RepositoryRef,
};

/// 契约里的时限：原意图挂满 30 分钟由生产者补「中途没了」，第一次不干净 1 小时后重查，轮转 24 小时。
const ABANDON_MS: i64 = 30 * 60_000;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

fn repository() -> RepositoryDigest {
    digest("rest:https://backup.example.test/repository/")
}

fn digest(location: &str) -> RepositoryDigest {
    RepositoryDigest::of(&RepositoryRef::parse(location).unwrap())
}

/// 派下来的原意图和它的回执（回执带真实的落账时刻和事件键）。
fn intent(answer: QueueAnswer) -> (CheckAssignment, AuditRecord) {
    let record = match answer {
        QueueAnswer::CheckAssigned(record) => *record,
        other => panic!("空闲又到点时必须派一组，实际：{other:?}"),
    };
    let Some(CheckEvent::Intent { assignment, target }) = read_check_event(record.event()) else {
        panic!("回执必须是抽查原意图");
    };
    assert_eq!(target, TARGET);
    (assignment, record)
}

fn assigned(stage: &Stage) -> CheckAssignment {
    intent(stage.ask(QueueCommand::RepositoryCheck)).0
}

/// 在给定时刻问一次「该不该抽查」：生产者按这个时刻判断，账本照样记真实的落账时刻。
fn ask_at(stage: &Stage, now_ms: i64) -> QueueAnswer {
    stage.producer.repository_check_with(|| Ok(now_ms)).unwrap()
}

fn report(stage: &Stage, key: CheckKey, outcome: CheckOutcome) -> Result<Vec<u8>, AuditError> {
    report_from(stage, key, outcome, repository())
}

fn report_from(
    stage: &Stage,
    key: CheckKey,
    outcome: CheckOutcome,
    repository: RepositoryDigest,
) -> Result<Vec<u8>, AuditError> {
    let report = RepositoryCheckReport::new(TARGET, key, outcome, repository).unwrap();
    stage.producer.dispatch(&report.encode().unwrap())
}

/// 账本里同一检查键的结果事件。
fn outcomes(stage: &Stage, intent: &AuditRecord) -> Vec<AuditRecord> {
    stage
        .assembled
        .human
        .query(
            &AuditFilter::default()
                .with_event_id(intent.event().event_id().clone())
                .with_phase(AuditPhase::Outcome),
        )
        .unwrap()
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_producer_assigns_the_first_bucket_and_records_exactly_the_reported_outcome() {
    let stage = Stage::new(true).await;
    let assignment = assigned(&stage);
    assert_eq!(assignment.key.sequence.get(), 1);
    assert_eq!(assignment.key.bucket, CheckBucket::FIRST);
    assert_eq!(assignment.purpose, CheckPurpose::Rotation);
    // 这一组还在查：不另派。
    assert_eq!(
        stage.ask(QueueCommand::RepositoryCheck),
        QueueAnswer::Outstanding
    );

    // 交「干净」：回执就是这份上报导出的那条结果；同一份重放回同一条。
    let request =
        RepositoryCheckReport::new(TARGET, assignment.key, CheckOutcome::clean(), repository())
            .unwrap();
    let first = request
        .decode_reply(&stage.producer.dispatch(&request.encode().unwrap()).unwrap())
        .unwrap();
    assert_eq!(first.event(), &request.event().unwrap());
    let again = request
        .decode_reply(&stage.producer.dispatch(&request.encode().unwrap()).unwrap())
        .unwrap();
    assert_eq!(again.offset(), first.offset(), "重放不追加第二条");
    // 同一检查换一个结果不落账。
    assert!(
        report(
            &stage,
            assignment.key,
            CheckOutcome::incomplete(IncompleteReason::Deadline)
        )
        .is_err()
    );
    // 刚查完一组：24 小时之内不再派。
    assert_eq!(
        stage.ask(QueueCommand::RepositoryCheck),
        QueueAnswer::NotDue
    );

    // 健康页读同一份账本：第 1 组读干净，首轮里正常。
    let checks = stage.producer.repository_checks(now_ms()).unwrap();
    assert_eq!(checks.verdict(), CheckVerdict::Healthy);
    assert!(checks.last_clean_ms[0].is_some());
    assert!(checks.last_clean_ms[1..].iter().all(Option::is_none));
    assert!(checks.running.is_none());
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reports_without_a_matching_intent_or_for_another_target_are_refused() {
    let stage = Stage::new(true).await;
    let never = CheckKey {
        sequence: NonZeroU64::new(9).unwrap(),
        bucket: CheckBucket::FIRST,
    };
    assert!(
        report(&stage, never, CheckOutcome::clean()).is_err(),
        "没有原意图的检查键不能落账"
    );
    let assignment = assigned(&stage);
    let wrong_bucket = CheckKey {
        sequence: assignment.key.sequence,
        bucket: assignment.key.bucket.following(),
    };
    assert!(
        report(&stage, wrong_bucket, CheckOutcome::clean()).is_err(),
        "同一序号换一组不落账"
    );
    let foreign = RepositoryCheckReport::new(
        BackupTargetKind::LocalTemporaryRepository,
        assignment.key,
        CheckOutcome::clean(),
        repository(),
    )
    .unwrap();
    assert!(
        stage.producer.dispatch(&foreign.encode().unwrap()).is_err(),
        "别的目标类别不落账"
    );
    // 上面都没落账：这一组仍在查。
    assert_eq!(
        stage.ask(QueueCommand::RepositoryCheck),
        QueueAnswer::Outstanding
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_backup_in_flight_comes_before_any_repository_check() {
    let stage = Stage::new(true).await;
    assert_eq!(stage.ask(QueueCommand::Schedule), QueueAnswer::Started);
    assert_eq!(
        stage.ask(QueueCommand::RepositoryCheck),
        QueueAnswer::Outstanding,
        "在封存或有作业没收口时不派抽查"
    );
    stage.producer.settle();
    stage.assembled.writable(true);
    assert!(matches!(
        stage.ask(QueueCommand::NextPending),
        QueueAnswer::Pending(_)
    ));
    assert_eq!(
        stage.ask(QueueCommand::RepositoryCheck),
        QueueAnswer::Outstanding,
        "待办作业没收口前不派抽查"
    );
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_intent_left_for_thirty_minutes_is_closed_by_the_producer_and_rechecked_an_hour_later() {
    let stage = Stage::new(true).await;
    let (assignment, record) = intent(stage.ask(QueueCommand::RepositoryCheck));
    let since = record.recorded_at().as_millisecond();

    // 差 1 毫秒满 30 分钟：还算在查，不补记、不另派。
    assert_eq!(
        ask_at(&stage, since + ABANDON_MS - 1),
        QueueAnswer::Outstanding
    );
    assert!(outcomes(&stage, &record).is_empty());

    // 满 30 分钟：生产者补一条「没做完（worker 中途没了）」，记的是生产者；刚补完不到 1 小时，不派。
    assert_eq!(ask_at(&stage, since + ABANDON_MS), QueueAnswer::NotDue);
    let closed = outcomes(&stage, &record);
    assert_eq!(closed.len(), 1);
    let Some(CheckEvent::Outcome {
        key,
        outcome,
        recorder,
    }) = read_check_event(closed[0].event())
    else {
        panic!("补记的必须是抽查结果事件");
    };
    assert_eq!(key, assignment.key);
    assert_eq!(
        outcome,
        CheckOutcome::incomplete(IncompleteReason::Abandoned)
    );
    assert_eq!(recorder, CheckRecorder::Producer);
    // 再问一次不补第二条。
    assert_eq!(ask_at(&stage, since + ABANDON_MS), QueueAnswer::NotDue);
    assert_eq!(outcomes(&stage, &record).len(), 1);

    // worker 迟到的结果：同一检查已经有结果，不落账。
    assert!(report(&stage, assignment.key, CheckOutcome::clean()).is_err());
    assert_eq!(outcomes(&stage, &record).len(), 1);

    // 健康页：最近一次是这条没做完，还没有干净的检查，不算在查。
    let checks = stage
        .producer
        .repository_checks(since + ABANDON_MS)
        .unwrap();
    assert!(checks.running.is_none());
    assert!(checks.last_clean_ms.iter().all(Option::is_none));
    let latest = checks.latest.unwrap();
    assert_eq!(latest.reason, Some(IncompleteReason::Abandoned));

    // 第一次不干净满 1 小时（从补记落账算起）：用新序号重查同一组。
    let closed_at = closed[0].recorded_at().as_millisecond();
    assert_eq!(ask_at(&stage, closed_at + HOUR_MS - 1), QueueAnswer::NotDue);
    let (retry, _) = intent(ask_at(&stage, closed_at + HOUR_MS));
    assert_eq!(retry.key.bucket, assignment.key.bucket);
    assert_eq!(retry.key.sequence.get(), assignment.key.sequence.get() + 1);
    assert_eq!(retry.purpose, CheckPurpose::Retry);
    stage.assembled.workbench.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_result_from_another_repository_forgets_the_buckets_read_before() {
    let stage = Stage::new(true).await;
    let (first, record) = intent(stage.ask(QueueCommand::RepositoryCheck));
    report(&stage, first.key, CheckOutcome::clean()).unwrap();

    // 满 24 小时轮到下一组。
    let rotate_at = record.recorded_at().as_millisecond() + DAY_MS;
    assert_eq!(ask_at(&stage, rotate_at - 1), QueueAnswer::NotDue);
    let (second, _) = intent(ask_at(&stage, rotate_at));
    assert_eq!(second.key.bucket, first.key.bucket.following());
    assert_eq!(second.purpose, CheckPurpose::Rotation);

    // 第二组报的是另一个仓库：第一组在旧仓库读干净的结果不再算，首轮覆盖从这一刻重新起算。
    report_from(
        &stage,
        second.key,
        CheckOutcome::clean(),
        digest("rest:https://backup.example.test/another/"),
    )
    .unwrap();
    let checks = stage.producer.repository_checks(rotate_at).unwrap();
    assert!(checks.last_clean_ms[0].is_none(), "旧仓库第 1 组的结果作废");
    assert!(checks.last_clean_ms[1].is_some());
    assert_eq!(checks.first_cycle_done(), Some(false));
    assert_eq!(checks.verdict(), CheckVerdict::Healthy);
    stage.assembled.workbench.shutdown().await.unwrap();
}
