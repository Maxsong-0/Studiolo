use super::*;
fn maintain(broker: &mut FsVaultBroker, decision: MaintenanceDecision) {
    flush(broker);
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    assert!(target.file_operation_id.is_some());
    let ticket = broker
        .prepare_maintenance(
            &target.review_id,
            decision,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    assert_eq!(ticket.intent().schema_version, Some(2));
    broker
        .confirm_maintenance_intent(&ticket, 3, &"b".repeat(64))
        .unwrap();
    broker.commit_maintenance(ticket).unwrap();
    let original = broker.pending_file_operation_audit(64).unwrap();
    assert_eq!(original.len(), 1);
    let maintenance = broker.pending_maintenance_audit(64).unwrap().remove(0);
    assert!(matches!(
        broker.confirm_maintenance_audit(maintenance, 4, &"b".repeat(64)),
        Err(BrokerError::MutationUnresolved)
    ));
    flush(broker);
    for candidate in broker.pending_maintenance_audit(64).unwrap() {
        broker
            .confirm_maintenance_audit(candidate, 4, &"b".repeat(64))
            .unwrap();
    }
    assert!(!broker.write_status().unwrap().unresolved);
}
#[test]
fn lost_preparing_and_prepared_tickets_can_cancel_via_real_maintenance_protocol() {
    for point in [50, 51] {
        let fixture = Fixture::new();
        let (mut broker, original) = fixture.seed(b"old");
        let request = delete(original, "lost");
        fault(&broker, point);
        assert!(
            broker
                .prepare_file_operation(request.clone(), metadata())
                .is_err()
        );
        drop(broker);
        let mut broker = fixture.reopen();
        maintain(&mut broker, MaintenanceDecision::CancelPrepared);
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
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
    }
}
#[test]
fn preserve_issued_distinguishes_present_missing_and_unobservable_current() {
    for scenario in [0, 1, 2] {
        let fixture = Fixture::new();
        let (mut broker, original) = fixture.seed(b"old");
        let request = delete(original, "unknown");
        let ticket = broker
            .prepare_file_operation(request.clone(), metadata())
            .unwrap();
        intent(&mut broker, &ticket);
        fault(&broker, if scenario == 0 { 52 } else { 54 });
        assert!(broker.commit_file_operation(&ticket).is_err());
        if scenario == 2 {
            std::os::unix::fs::symlink("/unreadable-outside", fixture.root.join("note.md"))
                .unwrap();
        }
        drop(broker);
        let mut broker = fixture.reopen();
        let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
        match scenario {
            0 => assert!(matches!(target.current, MaintenanceCurrent::Present { .. })),
            1 => assert_eq!(target.current, MaintenanceCurrent::Missing),
            _ => assert!(matches!(
                target.current,
                MaintenanceCurrent::Unobservable { .. }
            )),
        }
        maintain(&mut broker, MaintenanceDecision::PreserveInstalling);
        let result = broker
            .file_operation_replay(&request, &metadata())
            .unwrap()
            .unwrap()
            .receipt
            .unwrap();
        assert_eq!(result.outcome, FileOperationOutcome::Indeterminate);
        assert!(result.evidence_id.is_some());
        assert_eq!(broker.write_status().unwrap().committed_through, 0);
    }
}
#[test]
fn abandoned_maintenance_can_cancel_its_own_slot_after_original_target_becomes_unsafe() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let _original = broker
        .prepare_file_operation(delete(original, "pending"), metadata())
        .unwrap();
    flush(&mut broker);
    let review = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    let ticket = broker
        .prepare_maintenance(
            &review.review_id,
            MaintenanceDecision::CancelPrepared,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    broker
        .confirm_maintenance_intent(&ticket, 3, &"b".repeat(64))
        .unwrap();
    fs::remove_file(fixture.root.join("note.md")).unwrap();
    std::os::unix::fs::symlink("/unreadable-outside", fixture.root.join("note.md")).unwrap();
    let pending = broker.maintenance_pending().unwrap().unwrap();
    broker
        .cancel_maintenance(
            &pending.maintenance_id,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    for candidate in broker.pending_maintenance_audit(64).unwrap() {
        broker
            .confirm_maintenance_audit(candidate, 4, &"b".repeat(64))
            .unwrap();
    }
    assert!(broker.file_operation_status().unwrap().is_some());
    assert!(broker.write_status().unwrap().unresolved);
}
