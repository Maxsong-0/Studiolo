use super::super::support::SaveFault;
use super::*;
use std::sync::atomic::Ordering;
use studiolo_backup::worker::state::{JobEvent, UnknownStage};

fn state(world: &World) -> studiolo_backup::store::model::PersistedState {
    world.store.load_state().unwrap().unwrap()
}

#[test]
fn lost_final_ack_reopens_the_original_draft_without_reading_or_reuploading_expired_source() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2000);
    let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).unwrap();
    *world.finalizer.lose_ack.lock().unwrap() = true;
    assert_eq!(
        world.run(&worker, &key, &runner, &probe).unwrap(),
        JobOutcome::ResultUnknown(UnknownStage::FinalManifestAck)
    );
    let before = state(&world);
    assert!(before.pending_final.as_ref().unwrap().receipt.is_none());
    assert!(before.last_scheduled.is_none());
    world.clock.advance(100000);
    let reopened = world.worker(BackupTargetKind::LocalTemporaryRepository);
    // 此 API 没有 source / runner / data probe 参数，不会给过期的正文读取续期。
    assert_eq!(
        reopened.reconcile_final(&key, &world.finalizer).unwrap(),
        JobOutcome::Succeeded
    );
    assert_eq!(runner.seen.lock().unwrap().len(), 1);
    assert_eq!(probe.probed.lock().unwrap().len(), 1);
    assert_eq!(world.finalizer.commits.load(Ordering::SeqCst), 1);
    assert_eq!(state(&world).next_sequence, 2);
    assert_eq!(
        state(&world).last_final.as_ref().unwrap().draft(),
        &before.pending_final.unwrap().draft
    );
}

#[test]
fn missing_final_record_is_unknown_before_expiry_and_a_finalization_failure_after_expiry() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2000);
    let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).unwrap();
    *world.finalizer.omit_write.lock().unwrap() = true;
    world.run(&worker, &key, &runner, &probe).unwrap();
    *world.finalizer.absent_at.lock().unwrap() = 5000;
    assert_eq!(
        worker.reconcile_final(&key, &world.finalizer).unwrap(),
        JobOutcome::ResultUnknown(UnknownStage::FinalManifestAck)
    );
    *world.finalizer.read_error.lock().unwrap() = true;
    *world.finalizer.absent_at.lock().unwrap() = 10000;
    assert_eq!(
        worker.reconcile_final(&key, &world.finalizer).unwrap(),
        JobOutcome::ResultUnknown(UnknownStage::FinalManifestAck)
    );
    *world.finalizer.read_error.lock().unwrap() = false;
    assert_eq!(
        worker.reconcile_final(&key, &world.finalizer).unwrap(),
        JobOutcome::Failed(FailureReason::FinalManifestCommitFailed)
    );
    assert!(state(&world).last_scheduled.is_none());
    assert!(state(&world).pending_final.is_none());
    assert_eq!(world.finalizer.commits.load(Ordering::SeqCst), 1);
}

#[test]
fn no_final_write_occurs_before_the_draft_is_durable_and_upload_recovery_never_reuploads() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2000);
    let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).unwrap();
    *world.store.fail_state.lock().unwrap() = Some(SaveFault::Draft);
    assert_eq!(
        world.run(&worker, &key, &runner, &probe),
        Err(WorkerError::Store(StoreError::OutOfSpace))
    );
    assert_eq!(world.finalizer.commits.load(Ordering::SeqCst), 0);
    assert_eq!(state(&world).jobs[0].state, JobState::Verifying);
    worker.recover_interrupted().unwrap();
    assert_eq!(
        worker
            .reconcile_upload(&key, &world.seal, &probe, &world.finalizer)
            .unwrap(),
        JobOutcome::Succeeded
    );
    assert_eq!(runner.seen.lock().unwrap().len(), 1);
    assert_eq!(world.finalizer.commits.load(Ordering::SeqCst), 1);
}

#[test]
fn a_lost_local_receipt_write_is_recovered_from_the_remote_marker_not_reuploaded() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2000);
    let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).unwrap();
    *world.store.fail_state.lock().unwrap() = Some(SaveFault::Receipt);
    assert_eq!(
        world.run(&worker, &key, &runner, &probe),
        Err(WorkerError::Store(StoreError::OutOfSpace))
    );
    assert_eq!(
        state(&world).jobs[0].state,
        JobState::CommittingFinalManifest
    );
    assert_eq!(world.ledger.records().len(), 1);
    worker.recover_interrupted().unwrap();
    assert_eq!(
        worker.reconcile_final(&key, &world.finalizer).unwrap(),
        JobOutcome::Succeeded
    );
    assert_eq!(world.finalizer.commits.load(Ordering::SeqCst), 1);
}

#[test]
fn audit_ack_then_local_write_failure_keeps_a_stable_receipt_and_one_atomic_success_pointer() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2000);
    let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).unwrap();
    *world.store.fail_state.lock().unwrap() = Some(SaveFault::Succeeded);
    assert_eq!(
        world.run(&worker, &key, &runner, &probe),
        Err(WorkerError::Store(StoreError::OutOfSpace))
    );
    let pending = state(&world);
    assert!(matches!(
        pending.jobs[0].state,
        JobState::AuditPending { .. }
    ));
    assert!(pending.last_scheduled.is_none());
    assert_eq!(world.ledger.records().len(), 2);
    assert_eq!(
        worker.backfill_outcome(&key).unwrap(),
        JobOutcome::Succeeded
    );
    assert_eq!(world.ledger.records().len(), 2, "原 Outcome 幂等补记");
    assert_eq!(
        state(&world).last_final,
        pending.pending_final.unwrap().receipt
    );
    for saved in world.store.writes.lock().unwrap().iter() {
        assert_eq!(
            saved.jobs.iter().any(|j| j.state == JobState::Succeeded),
            saved.last_scheduled.is_some()
        );
        saved.validate().unwrap();
    }
}

#[test]
fn a_valid_audit_record_for_a_different_event_cannot_advance_success() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2000);
    let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).unwrap();
    *world.ledger.wrong_ack.lock().unwrap() = Some(AuditPhase::Outcome);
    assert_eq!(
        world.run(&worker, &key, &runner, &probe).unwrap(),
        JobOutcome::AuditPending
    );
    assert!(state(&world).last_scheduled.is_none());
    *world.ledger.wrong_ack.lock().unwrap() = None;
    assert_eq!(
        worker.backfill_outcome(&key).unwrap(),
        JobOutcome::Succeeded
    );
}

#[test]
fn a_second_job_cannot_overwrite_the_first_jobs_pending_final_evidence() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2000);
    let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
    let first = worker.enqueue(&checkpoint()).unwrap();
    *world.finalizer.lose_ack.lock().unwrap() = true;
    world.run(&worker, &first, &runner, &probe).unwrap();
    let second = worker.enqueue(&checkpoint()).unwrap();
    assert_eq!(
        world.run(&worker, &second, &runner, &probe),
        Err(WorkerError::JobBusy)
    );
    assert_eq!(
        state(&world).pending_final.as_ref().unwrap().draft.key(),
        &first
    );
    assert_eq!(runner.seen.lock().unwrap().len(), 1);
}

#[test]
fn queued_jobs_cannot_skip_to_an_unknown_upload_or_final_commit_and_capacity_precedes_audit() {
    let world = World::new();
    let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).unwrap();
    for stage in [
        UnknownStage::Sealing,
        UnknownStage::Uploading,
        UnknownStage::Verifying,
        UnknownStage::FinalManifestAck,
    ] {
        assert!(worker.mark_unknown(&key, stage).is_err());
    }
    for _ in 1..studiolo_backup::store::model::MAX_QUEUED {
        worker.enqueue(&checkpoint()).unwrap();
    }
    let events = world.ledger.records().len();
    assert_eq!(
        worker.enqueue(&checkpoint()),
        Err(WorkerError::Store(StoreError::LimitExceeded))
    );
    assert_eq!(world.ledger.records().len(), events);
}

#[test]
fn persisted_state_rejects_duplicate_sequences_wrong_stamps_and_receipt_detachment() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2000);
    let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).unwrap();
    world.run(&worker, &key, &runner, &probe).unwrap();
    let good = state(&world);
    for bad in 0..7 {
        let mut copy = good.clone();
        match bad {
            0 => copy.jobs.push(copy.jobs[0].clone()),
            1 => copy.last_scheduled = None,
            2 => copy.jobs[0].finalized_at = Some(InstantMillis::from_millis(1)),
            3 => copy.jobs[0].source = None,
            4 => copy.last_final = None,
            5 => {
                copy.last_scheduled.as_mut().unwrap().remote_verified_at =
                    InstantMillis::from_millis(2001)
            }
            _ => copy.jobs[0].state = JobState::Queued,
        }
        assert!(copy.validate().is_err(), "bad={bad}");
    }
    let mut queued = studiolo_backup::worker::state::JobRecord::queued(key);
    let original = queued.clone();
    assert!(
        queued
            .advance(JobEvent::StartSealing {
                checkpoint_at: InstantMillis::from_millis(-1)
            })
            .is_err()
    );
    assert_eq!(queued, original, "非法事件不能留下部分时间戳");
}
