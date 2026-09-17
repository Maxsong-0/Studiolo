use super::*;
#[test]
fn prepared_needs_both_audit_acks_and_keeps_original_permanent_cancel() {
    let fixture = Fixture::new();
    let (mut broker, request) = fixture.pending(1);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    let event = ticket.intent().clone();
    assert!(matches!(
        broker.commit_maintenance(ticket),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(fixture.body(), b"old");
    let ticket = broker
        .prepare_maintenance(&event.target.review_id, event.decision, actor())
        .unwrap();
    intent(&mut broker, &ticket);
    let receipt = broker.commit_maintenance(ticket).unwrap();
    assert_eq!(receipt.event.outcome, Some(MaintenanceOutcome::Applied));
    assert_eq!(
        receipt.event.resolution,
        Some(crate::ReconciliationOutcome::Cancelled)
    );
    assert_eq!(receipt.committed_through, 1);
    assert_eq!(broker.write_status().unwrap().phase, WriterPhase::Ready);
    assert!(broker.write_status().unwrap().unresolved);
    assert!(!broker.write_status().unwrap().read_blocked);
    assert!(matches!(
        broker.mutation_replay(&request),
        Err(BrokerError::MutationCancelled)
    ));
    assert!(matches!(
        broker.mutate(Mutation::create(
            VaultPath::parse("other.md").unwrap(),
            vec![],
            "other"
        )),
        Err(BrokerError::MutationUnresolved)
    ));
    flush(&mut broker);
    assert!(!broker.write_status().unwrap().unresolved);
    assert_eq!(fixture.body(), b"old");
    drop(broker);
    let broker = fixture.reopen();
    assert!(!broker.write_status().unwrap().unresolved);
    assert!(broker.pending_maintenance_audit(64).unwrap().is_empty());
}
#[test]
fn installing_preserves_three_real_byte_versions_without_claiming_success() {
    let fixture = Fixture::new();
    let (mut broker, request) = fixture.pending(3);
    fs::write(fixture.root.join("note.md"), b"outside").unwrap();
    let ticket = prepare(&mut broker, MaintenanceDecision::PreserveInstalling);
    assert!(ticket.intent().target.proposed_available);
    intent(&mut broker, &ticket);
    let receipt = broker.commit_maintenance(ticket).unwrap();
    let id = receipt.event.evidence_id.unwrap();
    for (role, expected) in [
        ("previous", b"old".as_slice()),
        ("current", b"outside"),
        ("proposed", b"proposed"),
    ] {
        assert_eq!(
            fs::read(
                fixture
                    .data
                    .join("snapshots")
                    .join(format!("recovery-{id}-{role}.bin"))
            )
            .unwrap(),
            expected
        );
    }
    assert_eq!(fixture.body(), b"outside");
    assert!(matches!(
        broker.mutation_replay(&request),
        Err(BrokerError::MutationIndeterminate { .. })
    ));
    flush(&mut broker);
    assert!(!broker.write_status().unwrap().unresolved);
}
#[test]
fn cancelling_maintenance_keeps_original_prepared_even_after_target_changed() {
    let fixture = Fixture::new();
    let (mut broker, request) = fixture.pending(1);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    intent(&mut broker, &ticket);
    fs::write(fixture.root.join("note.md"), b"outside").unwrap();
    let receipt = broker
        .cancel_maintenance(&ticket.intent().event_id, actor())
        .unwrap();
    assert_eq!(receipt.event.outcome, Some(MaintenanceOutcome::Cancelled));
    assert_eq!(receipt.event.resolution, None);
    assert!(broker.mutation_replay(&request).unwrap().is_none());
    flush(&mut broker);
    assert_eq!(broker.write_status().unwrap().phase, WriterPhase::Prepared);
    assert!(broker.write_status().unwrap().unresolved);
    assert_eq!(fixture.body(), b"outside");
}
#[test]
fn stale_review_before_execution_has_own_not_applied_outcome_only() {
    let fixture = Fixture::new();
    let (mut broker, request) = fixture.pending(1);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    intent(&mut broker, &ticket);
    let original = fixture.body();
    fs::write(fixture.root.join("note.md"), b"changed").unwrap();
    fs::write(fixture.root.join("note.md"), original).unwrap();
    let receipt = broker.commit_maintenance(ticket).unwrap();
    assert_eq!(receipt.event.outcome, Some(MaintenanceOutcome::NotApplied));
    assert_eq!(receipt.event.resolution, None);
    assert!(broker.mutation_replay(&request).unwrap().is_none());
}
#[test]
fn pending_proposal_missing_is_not_replaced_by_current_hash_or_user_wish() {
    let fixture = Fixture::new();
    let (mut broker, _) = fixture.pending(3);
    let revision = broker
        .pending_mutation()
        .unwrap()
        .unwrap()
        .target_revision();
    fs::remove_file(fixture.data.join("history").join(revision.to_string())).unwrap();
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    assert!(!target.proposed_available);
    assert!(matches!(
        broker.prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::PreserveInstalling,
            actor()
        ),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(fixture.body(), b"proposed");
    assert!(!fixture.data.join("maintenance-v1").exists());
}
