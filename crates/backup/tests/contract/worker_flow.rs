//! Worker 端到端契约：Intent 先持久、审计失败关闭；
//! 上传中断、ACK 丢失、Outcome 审计失败、磁盘满与目标未接入各自留下不同的真实状态。

use super::support::{
    MemoryStore, PickyLedger, StubProbe, StubRunner, TestClock, manifest_inputs, ticket,
};
use studiolo_audit_ledger::AuditPhase;
use studiolo_backup::store::model::{BackupStateStore, StoreError};
use studiolo_backup::worker::job::{BackupWorker, JobOutcome, WorkerError};
use studiolo_backup::worker::recovery_point::{BackupTargetKind, RecoveryPointDisplay, RpoBudget};
use studiolo_backup::worker::restic::{CredentialHolder, RepositoryRef, UnverifiedReason};
use studiolo_backup::worker::state::{FailureReason, JobState, UnknownStage};
use studiolo_core::VaultUuid;
use studiolo_core::checkpoint::CheckpointId;
use tempfile::TempDir;

fn repository() -> RepositoryRef {
    RepositoryRef::parse("/srv/backup/repo").expect("repository")
}

fn checkpoint() -> CheckpointId {
    CheckpointId::parse("cp-0001").expect("checkpoint id")
}

struct World {
    ledger: PickyLedger,
    store: MemoryStore,
    clock: TestClock,
    temporary: TempDir,
    vault: VaultUuid,
    pair: VaultUuid,
}

impl World {
    fn new() -> Self {
        Self {
            ledger: PickyLedger::new(),
            store: MemoryStore::default(),
            clock: TestClock::new(3_000),
            temporary: TempDir::new().expect("tempdir"),
            vault: VaultUuid::new(),
            pair: VaultUuid::new(),
        }
    }

    fn worker<'a>(
        &'a self,
        runner: &'a StubRunner,
        probe: &'a StubProbe,
        target: BackupTargetKind,
    ) -> BackupWorker<'a> {
        BackupWorker::new(
            &self.ledger,
            runner,
            probe,
            &self.store,
            repository(),
            CredentialHolder::BackupWorker,
            target,
        )
        .expect("worker")
    }
}

#[test]
fn the_append_credential_cannot_be_handed_to_the_app_or_any_owner() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2_000);
    for holder in [
        CredentialHolder::App,
        CredentialHolder::Broker,
        CredentialHolder::Auth,
        CredentialHolder::Audit,
        CredentialHolder::TaskEngine,
    ] {
        assert_eq!(
            BackupWorker::new(
                &world.ledger,
                &runner,
                &probe,
                &world.store,
                repository(),
                holder,
                BackupTargetKind::LocalTemporaryRepository,
            )
            .err(),
            Some(WorkerError::CredentialHolderForbidden)
        );
    }
}

#[test]
fn enqueueing_persists_the_intent_before_the_job_record_and_fails_closed_on_audit_failure() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2_000);
    let worker = world.worker(&runner, &probe, BackupTargetKind::LocalTemporaryRepository);
    world.ledger.fail(AuditPhase::Intent);

    let error = worker.enqueue(&checkpoint()).err();

    assert!(matches!(error, Some(WorkerError::Audit(_))));
    assert_eq!(
        world.store.load_state().expect("load"),
        None,
        "Intent 没落盘就不能留下排队记录"
    );

    world.ledger.recover();
    let key = worker.enqueue(&checkpoint()).expect("enqueue");
    assert_eq!(key.sequence.get(), 1);
    let records = world.ledger.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[0].event().action(), "backup.snapshot");
}

#[test]
fn a_full_run_reaches_success_and_publishes_a_recovery_point() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2_000);
    let worker = world.worker(&runner, &probe, BackupTargetKind::DedicatedAppendOnlyTarget);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");

    let outcome = worker
        .run(
            &key,
            &ticket(world.vault, world.pair, world.temporary.path()),
            manifest_inputs(),
            &world.clock,
        )
        .expect("run");

    assert_eq!(outcome, JobOutcome::Succeeded);
    let state = world.store.load_state().expect("load").expect("state");
    assert_eq!(state.jobs[0].state, JobState::Succeeded);
    let stamps = state.last_scheduled.expect("recovery point");
    assert_eq!(stamps.checkpoint_at.millis(), 1_000);
    assert_eq!(stamps.remote_verified_at.millis(), 2_000);
    assert_eq!(stamps.finalized_at.millis(), 3_000);
    assert!(world.store.load_final_manifest().expect("load").is_some());
    assert_eq!(world.ledger.records().len(), 2);

    // 首次完整 seed 校验之前，界面仍然显示「尚未建立灾难恢复点」。
    let before = worker
        .recovery_point_status(Some(&world.clock), RpoBudget::DEFAULT, true)
        .expect("status");
    assert_eq!(before.scheduled, RecoveryPointDisplay::NotEstablished);
    worker.mark_seed_verified().expect("seed verified");
    let after = worker
        .recovery_point_status(Some(&world.clock), RpoBudget::DEFAULT, true)
        .expect("status");
    assert!(after.scheduled.is_established());
    assert!(after.shows_immutable_protection());
}

#[test]
fn seed_verification_is_refused_while_nothing_has_succeeded() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2_000);
    let worker = world.worker(&runner, &probe, BackupTargetKind::LocalTemporaryRepository);
    worker.enqueue(&checkpoint()).expect("enqueue");

    assert_eq!(
        worker.mark_seed_verified().err(),
        Some(WorkerError::SeedNotVerifiable)
    );
}

#[test]
fn an_upload_that_exits_zero_but_leaves_nothing_remote_is_not_a_recovery_point() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::absent();
    let worker = world.worker(&runner, &probe, BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");

    let outcome = worker
        .run(
            &key,
            &ticket(world.vault, world.pair, world.temporary.path()),
            manifest_inputs(),
            &world.clock,
        )
        .expect("run");

    assert_eq!(
        outcome,
        JobOutcome::Failed(FailureReason::RemoteVerificationFailed)
    );
    let state = world.store.load_state().expect("load").expect("state");
    assert!(state.last_scheduled.is_none());
    assert!(world.store.load_final_manifest().expect("load").is_none());
    assert_eq!(
        probe.probed.lock().expect("probe lock").as_slice(),
        [key.sequence.get()],
        "必须沿原作业键去远端核对"
    );
}

#[test]
fn a_rejected_upload_is_recorded_as_a_failure_with_its_own_outcome_audit() {
    let world = World::new();
    let runner = StubRunner::exiting(false);
    let probe = StubProbe::present(2_000);
    let worker = world.worker(&runner, &probe, BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");

    let outcome = worker
        .run(
            &key,
            &ticket(world.vault, world.pair, world.temporary.path()),
            manifest_inputs(),
            &world.clock,
        )
        .expect("run");

    assert_eq!(outcome, JobOutcome::Failed(FailureReason::UploadRejected));
    let records = world.ledger.records();
    assert_eq!(records.len(), 2);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
}

#[test]
fn a_remote_that_is_confirmed_while_the_outcome_audit_fails_stays_audit_pending() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2_000);
    let worker = world.worker(&runner, &probe, BackupTargetKind::DedicatedAppendOnlyTarget);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");
    world.ledger.fail(AuditPhase::Outcome);

    let outcome = worker
        .run(
            &key,
            &ticket(world.vault, world.pair, world.temporary.path()),
            manifest_inputs(),
            &world.clock,
        )
        .expect("run");

    assert_eq!(outcome, JobOutcome::AuditPending);
    let state = world.store.load_state().expect("load").expect("state");
    assert!(
        matches!(state.jobs[0].state, JobState::AuditPending { .. }),
        "远端已确认的真实状态必须保留"
    );
    assert!(state.last_scheduled.is_none(), "成功指针不能在补记之前前移");
    assert!(
        world.store.load_final_manifest().expect("load").is_some(),
        "不能声称上传没发生"
    );

    world.ledger.recover();
    assert_eq!(
        worker
            .backfill_outcome(&key, &world.clock)
            .expect("backfill"),
        JobOutcome::Succeeded
    );
    let settled = world.store.load_state().expect("load").expect("state");
    assert_eq!(settled.jobs[0].state, JobState::Succeeded);
    let stamps = settled.last_scheduled.expect("recovery point");
    assert_eq!(stamps.remote_verified_at.millis(), 2_000);
}

#[test]
fn the_outcome_payload_states_that_it_happened_after_the_checkpoint_cutoff() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2_000);
    let worker = world.worker(&runner, &probe, BackupTargetKind::DedicatedAppendOnlyTarget);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");
    worker
        .run(
            &key,
            &ticket(world.vault, world.pair, world.temporary.path()),
            manifest_inputs(),
            &world.clock,
        )
        .expect("run");

    let records = world.ledger.records();
    let outcome = records
        .iter()
        .find(|record| record.event().phase() == AuditPhase::Outcome)
        .expect("outcome");
    assert_eq!(
        outcome.event().payload().as_value()["recorded_after_checkpoint_cutoff"],
        serde_json::json!(true)
    );
    let encoded = serde_json::to_string(outcome.event()).expect("encode");
    assert!(!encoded.contains("/srv/backup/repo"), "payload 不放原配置");
}

#[test]
fn a_full_disk_during_the_final_manifest_commit_is_a_named_failure() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2_000);
    let worker = world.worker(&runner, &probe, BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");
    *world.store.fail_manifest_commit.lock().expect("fail lock") = Some(StoreError::OutOfSpace);

    let outcome = worker
        .run(
            &key,
            &ticket(world.vault, world.pair, world.temporary.path()),
            manifest_inputs(),
            &world.clock,
        )
        .expect("run");

    assert_eq!(outcome, JobOutcome::Failed(FailureReason::OutOfSpace));
    let state = world.store.load_state().expect("load").expect("state");
    assert!(state.last_scheduled.is_none());
}

#[test]
fn a_target_that_is_not_connected_reports_unverified_rather_than_success_or_failure() {
    let world = World::new();
    let runner = StubRunner::unverified(UnverifiedReason::NoRealRepository);
    let probe = StubProbe::absent();
    let worker = world.worker(&runner, &probe, BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");

    let outcome = worker
        .run(
            &key,
            &ticket(world.vault, world.pair, world.temporary.path()),
            manifest_inputs(),
            &world.clock,
        )
        .expect("run");

    assert_eq!(
        outcome,
        JobOutcome::Unverified(UnverifiedReason::NoRealRepository)
    );
    let state = world.store.load_state().expect("load").expect("state");
    assert_eq!(
        state.jobs[0].state,
        JobState::Failed {
            reason: FailureReason::TargetUnverified
        }
    );
    assert!(state.last_scheduled.is_none());
    let status = worker
        .recovery_point_status(Some(&world.clock), RpoBudget::DEFAULT, false)
        .expect("status");
    assert_eq!(status.scheduled, RecoveryPointDisplay::NotEstablished);
    assert!(!status.shows_immutable_protection());
}

#[test]
fn an_interrupted_run_is_reconciled_under_the_original_job_key() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2_000);
    let worker = world.worker(&runner, &probe, BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");
    worker
        .mark_unknown(&key, UnknownStage::Uploading)
        .expect("mark unknown");

    let state = worker.reconcile_unknown(&key).expect("reconcile");

    assert_eq!(state, JobState::CommittingFinalManifest);
    assert_eq!(
        probe.probed.lock().expect("probe lock").as_slice(),
        [key.sequence.get()]
    );
}

#[test]
fn a_lost_final_manifest_ack_settles_as_a_failure_when_the_remote_has_nothing() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::absent();
    let worker = world.worker(&runner, &probe, BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");
    worker
        .mark_unknown(&key, UnknownStage::FinalManifestAck)
        .expect("mark unknown");

    let state = worker.reconcile_unknown(&key).expect("reconcile");

    assert_eq!(
        state,
        JobState::Failed {
            reason: FailureReason::UploadRejected
        }
    );
}

#[test]
fn the_planned_restic_argument_vector_only_ever_points_at_the_transfer_source() {
    let world = World::new();
    let runner = StubRunner::exiting(true);
    let probe = StubProbe::present(2_000);
    let worker = world.worker(&runner, &probe, BackupTargetKind::LocalTemporaryRepository);
    let key = worker.enqueue(&checkpoint()).expect("enqueue");
    let seal = ticket(world.vault, world.pair, world.temporary.path());
    worker
        .run(&key, &seal, manifest_inputs(), &world.clock)
        .expect("run");

    let seen = runner.seen.lock().expect("seen lock");
    assert_eq!(seen.len(), 1);
    assert!(seen[0].contains(&seal.transfer_source.root.display().to_string()));
    assert!(seen[0].contains(&"--one-file-system".to_owned()));
}
