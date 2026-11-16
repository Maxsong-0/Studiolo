use super::*;

#[test]
fn failed_and_unverified_runs_retain_the_exact_outcome_until_audit_recovers() {
    for unverified in [false, true] {
        let world = World::new();
        let runner = if unverified {
            StubRunner::unverified(UnverifiedReason::TargetUnreachable)
        } else {
            StubRunner::exiting(false)
        };
        let probe = StubProbe::absent();
        let worker = world.worker(BackupTargetKind::LocalTemporaryRepository);
        let key = worker.enqueue(&checkpoint()).unwrap();
        world.ledger.fail(AuditPhase::Outcome);
        assert!(matches!(
            world.run(&worker, &key, &runner, &probe),
            Err(WorkerError::Audit(_))
        ));
        let pending = world
            .store
            .load_state()
            .unwrap()
            .unwrap()
            .pending_failure
            .unwrap();
        assert_eq!(pending.key, key);
        assert_eq!(
            pending.unverified,
            unverified.then_some(UnverifiedReason::TargetUnreachable)
        );
        let next = worker.enqueue(&checkpoint()).unwrap();
        assert_eq!(
            world.run(&worker, &next, &runner, &probe),
            Err(WorkerError::JobBusy)
        );
        world.ledger.recover();
        let expected = if unverified {
            JobOutcome::Unverified(UnverifiedReason::TargetUnreachable)
        } else {
            JobOutcome::Failed(FailureReason::UploadRejected)
        };
        assert_eq!(worker.backfill_failure(&key).unwrap(), expected);
        assert_eq!(world.ledger.records().len(), 3);
        assert!(
            world
                .store
                .load_state()
                .unwrap()
                .unwrap()
                .pending_failure
                .is_none()
        );
        assert_eq!(runner.seen.lock().unwrap().len(), 1);
        assert!(
            world
                .store
                .load_state()
                .unwrap()
                .unwrap()
                .last_scheduled
                .is_none()
        );
    }
}
