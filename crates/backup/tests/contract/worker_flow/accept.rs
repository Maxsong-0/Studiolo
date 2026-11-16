//! 序号由生产者发：worker 只照原键登记；来源过期有自己的失败出口。
use super::*;
use std::num::NonZeroU64;
use studiolo_audit_ledger::AuditLedger;
use studiolo_backup::worker::job::{JobPosition, backup_intent};
use studiolo_backup::worker::restic::{ResticError, ResticInvocation, ResticOutcome, ResticRunner};
use studiolo_backup::worker::state::{JobKey, UnknownStage};

const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;
fn issued(sequence: u64, checkpoint: &str) -> JobKey {
    JobKey {
        sequence: NonZeroU64::new(sequence).unwrap(),
        checkpoint_id: CheckpointId::parse(checkpoint).unwrap(),
    }
}
/// 生产者一侧做的事：在封存之前把原意图写进账本。
fn produce(world: &World, key: &JobKey) {
    world
        .ledger
        .append(backup_intent(key, TARGET).unwrap())
        .unwrap();
}
struct BrokenRunner;
impl ResticRunner for BrokenRunner {
    fn run(&self, _: &ResticInvocation) -> Result<ResticOutcome, ResticError> {
        Err(ResticError::ExecutionFailed)
    }
}

#[test]
fn a_producer_issued_key_is_accepted_only_when_its_original_intent_is_in_the_ledger() {
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = issued(5, "cp-0001");
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Absent);
    assert_eq!(
        worker.accept(&key).err(),
        Some(WorkerError::EvidenceMismatch),
        "账本里没有原意图就不登记"
    );
    assert_eq!(world.store.load_state().unwrap(), None);

    produce(&world, &key);
    worker.accept(&key).unwrap();
    worker.accept(&key).unwrap();
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Queued);
    let state = world.store.load_state().unwrap().unwrap();
    assert_eq!(state.jobs.len(), 1);
    assert_eq!(state.next_sequence, 6, "下一个序号跟着原键走");
    assert_eq!(world.ledger.records().len(), 1, "接收只读账本，不追加");
}

#[test]
fn an_older_sequence_or_a_reissued_checkpoint_is_refused() {
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = issued(5, "cp-0001");
    produce(&world, &key);
    worker.accept(&key).unwrap();
    for other in [issued(3, "cp-0000"), issued(9, "cp-0001")] {
        produce(&world, &other);
        assert_eq!(
            worker.accept(&other).err(),
            Some(WorkerError::EvidenceMismatch),
            "{other:?}"
        );
    }
    // 另一个目标类别的意图不是这台 worker 的作业。
    let foreign = issued(11, "cp-0002");
    world
        .ledger
        .append(backup_intent(&foreign, BackupTargetKind::LocalTemporaryRepository).unwrap())
        .unwrap();
    assert_eq!(
        worker.accept(&foreign).err(),
        Some(WorkerError::EvidenceMismatch)
    );
    assert_eq!(world.store.load_state().unwrap().unwrap().jobs.len(), 1);
}

#[test]
fn an_accepted_key_runs_through_the_unchanged_state_machine() {
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = issued(5, "cp-0001");
    produce(&world, &key);
    worker.accept(&key).unwrap();
    let outcome = world
        .run(
            &worker,
            &key,
            &StubRunner::exiting(true),
            &StubProbe::present(2_000),
        )
        .unwrap();
    assert_eq!(outcome, JobOutcome::Succeeded);
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Finished);
    assert_eq!(world.ledger.records().len(), 2, "原意图加成功结果");
}

#[test]
fn an_expired_source_fails_the_job_without_another_upload() {
    // 排队中：还什么都没做。
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = issued(5, "cp-0001");
    produce(&world, &key);
    assert_eq!(worker.expire(&key).err(), Some(WorkerError::JobNotFound));
    worker.accept(&key).unwrap();
    assert_eq!(
        worker.expire(&key).unwrap(),
        JobOutcome::Failed(FailureReason::SourceExpired)
    );
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Finished);
    let records = world.ledger.records();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[1].event().payload().as_value()["reason"],
        "source_expired"
    );

    // 上传结果未知：不重传、不猜远端，只记失败。
    let world = World::new();
    let worker = world.worker(TARGET);
    produce(&world, &key);
    worker.accept(&key).unwrap();
    let outcome = worker.run(
        &key,
        &world.seal,
        DataRemote {
            runner: &BrokenRunner,
            probe: &StubProbe::absent(),
        },
        &world.finalizer,
    );
    assert_eq!(
        outcome.unwrap(),
        JobOutcome::ResultUnknown(UnknownStage::Uploading)
    );
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Interrupted);
    assert_eq!(
        worker.expire(&key).unwrap(),
        JobOutcome::Failed(FailureReason::SourceExpired)
    );
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Finished);
}

#[test]
fn expiry_never_overrides_a_persisted_final_draft_and_a_lost_outcome_is_backfilled() {
    // 最终清单草稿已经持久：只能走非正文核对，过期出口拒绝。
    let world = World::new();
    let worker = world.worker(TARGET);
    let key = issued(5, "cp-0001");
    produce(&world, &key);
    worker.accept(&key).unwrap();
    *world.finalizer.lose_ack.lock().unwrap() = true;
    world
        .run(
            &worker,
            &key,
            &StubRunner::exiting(true),
            &StubProbe::present(2_000),
        )
        .unwrap();
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Finalizing);
    assert_eq!(
        worker.expire(&key).err(),
        Some(WorkerError::EvidenceMismatch)
    );

    // 失败结果写不进账本：失败事实留在本地，之后原样补记。
    let world = World::new();
    let worker = world.worker(TARGET);
    produce(&world, &key);
    worker.accept(&key).unwrap();
    world.ledger.fail(AuditPhase::Outcome);
    assert!(matches!(worker.expire(&key), Err(WorkerError::Audit(_))));
    assert_eq!(worker.position(&key).unwrap(), JobPosition::FailurePending);
    world.ledger.recover();
    assert_eq!(
        worker.backfill_failure(&key).unwrap(),
        JobOutcome::Failed(FailureReason::SourceExpired)
    );
    assert_eq!(worker.position(&key).unwrap(), JobPosition::Finished);
}
