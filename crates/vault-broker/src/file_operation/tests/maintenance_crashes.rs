use super::*;
#[test]
fn maintenance_crash_after_original_terminal_only_backfills_fixed_events() {
    for point in [33, 34, 35] {
        let fixture = Fixture::new();
        let (mut broker, original) = fixture.seed(b"old");
        let request = delete(original, "maintenance-crash");
        let _original = broker
            .prepare_file_operation(request.clone(), metadata())
            .unwrap();
        flush(&mut broker);
        let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
        let ticket = broker
            .prepare_maintenance(
                &target.review_id,
                MaintenanceDecision::CancelPrepared,
                MaintenanceActor::current().unwrap(),
            )
            .unwrap();
        broker
            .confirm_maintenance_intent(&ticket, 3, &"b".repeat(64))
            .unwrap();
        fault(&broker, point);
        assert!(broker.commit_maintenance(ticket).is_err());
        drop(broker);
        let mut broker = fixture.reopen();
        if point == 33 {
            assert!(
                broker
                    .file_operation_replay(&request, &metadata())
                    .unwrap()
                    .unwrap()
                    .receipt
                    .is_none()
            );
            let pending = broker.maintenance_pending().unwrap().unwrap();
            let ticket = broker
                .prepare_maintenance(
                    &pending.intent.target.review_id,
                    pending.intent.decision,
                    MaintenanceActor::current().unwrap(),
                )
                .unwrap();
            broker.commit_maintenance(ticket).unwrap();
        }
        flush(&mut broker);
        for candidate in broker.pending_maintenance_audit(64).unwrap() {
            broker
                .confirm_maintenance_audit(candidate, 4, &"b".repeat(64))
                .unwrap();
        }
        assert_eq!(
            broker
                .file_operation_replay(&request, &metadata())
                .unwrap()
                .unwrap()
                .receipt
                .unwrap()
                .outcome,
            FileOperationOutcome::Cancelled
        );
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
        assert!(!broker.write_status().unwrap().unresolved);
    }
}
