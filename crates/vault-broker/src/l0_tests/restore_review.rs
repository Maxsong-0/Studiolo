use super::*;

#[test]
fn installing_restore_requires_existing_recovery_and_never_reports_success_by_hash() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (source, expected) = versions(&mut broker);
    ack(&mut broker);
    let (request, metadata) = restore_request(source, expected, "restore");
    let ticket = broker.prepare_recorded_restore(request, metadata).unwrap();
    broker
        .confirm_recorded_restore_intent(&ticket, 1, &"a".repeat(64))
        .unwrap();
    interrupt(&broker, 3);
    assert!(broker.commit_recorded_restore(ticket).is_err());
    assert_eq!(fixture.note(), b"A");
    assert!(broker.pending_recorded_restores(10).unwrap().is_empty());
    assert!(
        broker
            .restore_replay(&path(), expected, source, "restore", &actor())
            .unwrap()
            .is_none()
    );
    let review = broker.pending_mutation().unwrap().unwrap();
    broker
        .reconcile_pending(
            &review,
            PendingDecision::PreserveCurrent {
                original_content: b"A".to_vec(),
            },
        )
        .unwrap();
    assert!(broker.content_audit_status().unwrap().restore_unconfirmed);
    assert!(matches!(
        broker.restore_replay(&path(), expected, source, "restore", &actor()),
        Err(BrokerError::MutationIndeterminate { .. })
    ));
    ack(&mut broker);
    assert!(!broker.content_audit_status().unwrap().restore_unconfirmed);
}

#[test]
fn strict_restore_review_rejects_cross_instance_and_journal_change() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (source, expected) = versions(&mut broker);
    let (request, metadata) = restore_request(source, expected, "restore");
    let ticket = broker.prepare_recorded_restore(request, metadata).unwrap();
    let review = broker.pending_recorded_restores(10).unwrap().remove(0);
    drop(broker);
    let mut broker = fixture.reopen();
    assert!(matches!(
        broker.cancel_recorded_restore(ticket),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(matches!(
        broker.cancel_pending_recorded_restore(review),
        Err(BrokerError::CaptureChanged)
    ));
    let review = broker.pending_recorded_restores(10).unwrap().remove(0);
    let mut state = crate::mutation::load(&broker).unwrap();
    state.next_sequence += 1;
    crate::mutation::save(&broker, &state).unwrap();
    assert!(broker.cancel_pending_recorded_restore(review).is_err());
}

#[test]
fn another_restore_cannot_enter_while_original_restore_is_unconfirmed() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (source, expected) = versions(&mut broker);
    let (request, metadata) = restore_request(source, expected, "restore");
    let ticket = broker
        .prepare_recorded_restore(request.clone(), metadata.clone())
        .unwrap();
    let (other, other_metadata) = restore_request(source, expected, "different");
    assert!(matches!(
        broker.prepare_recorded_restore(other, other_metadata),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(broker.prepare_recorded_restore(request, metadata).is_ok());
    broker.cancel_recorded_restore(ticket).unwrap();
    let (other, other_metadata) = restore_request(source, expected, "different");
    assert!(
        broker
            .prepare_recorded_restore(other, other_metadata)
            .is_ok()
    );
}
