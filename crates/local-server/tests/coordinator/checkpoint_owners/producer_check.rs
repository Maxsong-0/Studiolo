//! 生产者接真实账本派仓库抽查、收结果：只按账本定组号，结果要有同一检查键的原意图才落账。
use super::{producer::Stage, *};
use crate::backup::{BackupJobsProbe, CheckVerdict, producer::TARGET};
use studiolo_audit_ledger::AuditError;
use studiolo_backup::worker::{
    audit::{QueueAnswer, QueueCommand, RepositoryCheckReport},
    repository_check::{
        CheckAssignment, CheckBucket, CheckEvent, CheckKey, CheckOutcome, CheckPurpose,
        IncompleteReason, RepositoryDigest, read_check_event,
    },
    restic::RepositoryRef,
};

fn repository() -> RepositoryDigest {
    RepositoryDigest::of(
        &RepositoryRef::parse("rest:https://backup.example.test/repository/").unwrap(),
    )
}

fn assigned(stage: &Stage) -> CheckAssignment {
    let QueueAnswer::CheckAssigned(record) = stage.ask(QueueCommand::RepositoryCheck) else {
        panic!("空闲又到点时必须派一组");
    };
    let Some(CheckEvent::Intent { assignment, target }) = read_check_event(record.event()) else {
        panic!("回执必须是抽查原意图");
    };
    assert_eq!(target, TARGET);
    assignment
}

fn report(stage: &Stage, key: CheckKey, outcome: CheckOutcome) -> Result<Vec<u8>, AuditError> {
    let report = RepositoryCheckReport::new(TARGET, key, outcome, repository()).unwrap();
    stage.producer.dispatch(&report.encode().unwrap())
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
