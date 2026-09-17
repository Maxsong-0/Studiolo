use super::*;

#[test]
fn restore_cannot_bypass_real_intent_confirmation() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (source, expected) = versions(&mut broker);
    ack(&mut broker);
    let (request, metadata) = restore_request(source, expected, "restore");
    assert!(
        broker
            .mutate_recorded(request.clone(), metadata.clone())
            .is_err()
    );
    let ticket = broker
        .prepare_recorded_restore(request.clone(), metadata.clone())
        .unwrap();
    assert!(broker.content_audit_status().unwrap().restore_unconfirmed);
    assert_eq!(fixture.note(), b"B");
    assert!(broker.commit_recorded_restore(ticket).is_err());
    assert!(broker.mutate(request.clone()).is_err());
    let ticket = broker.prepare_recorded_restore(request, metadata).unwrap();
    broker
        .confirm_recorded_restore_intent(&ticket, 1, &"a".repeat(64))
        .unwrap();
    let receipt = broker.commit_recorded_restore(ticket).unwrap();
    assert_eq!(receipt.revision, source);
    assert_eq!(fixture.note(), b"A");
    assert!(broker.content_audit_status().unwrap().restore_unconfirmed);
    ack(&mut broker);
    assert!(!broker.content_audit_status().unwrap().restore_unconfirmed);
}

#[test]
fn lost_browser_key_can_be_reviewed_and_cancelled_after_reopen() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (source, expected) = versions(&mut broker);
    ack(&mut broker);
    let (request, metadata) = restore_request(source, expected, "restore");
    drop(
        broker
            .prepare_recorded_restore(request.clone(), metadata.clone())
            .unwrap(),
    );
    drop(broker);
    let mut broker = fixture.reopen();
    assert!(!broker.write_status().unwrap().unresolved);
    assert!(broker.content_audit_status().unwrap().restore_unconfirmed);
    let review = broker.pending_recorded_restores(10).unwrap().remove(0);
    assert_eq!(review.intent().path, path());
    broker.cancel_pending_recorded_restore(review).unwrap();
    assert_eq!(fixture.note(), b"B");
    assert!(!broker.content_audit_status().unwrap().restore_unconfirmed);
    assert_eq!(broker.content_audit_status().unwrap().pending, 2);
    assert!(matches!(
        broker.restore_replay(&path(), expected, source, "restore", &actor()),
        Err(BrokerError::MutationCancelled)
    ));
    assert!(matches!(
        broker.mutate(request),
        Err(BrokerError::MutationCancelled)
    ));
}

#[test]
fn cancelled_and_rejected_restores_do_not_freeze_normal_editing() {
    for cancelled in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        let (source, expected) = versions(&mut broker);
        ack(&mut broker);
        let (request, metadata) = restore_request(source, expected, "restore");
        let ticket = broker.prepare_recorded_restore(request, metadata).unwrap();
        if cancelled {
            broker.cancel_recorded_restore(ticket).unwrap();
        } else {
            broker
                .confirm_recorded_restore_intent(&ticket, 1, &"a".repeat(64))
                .unwrap();
            fs::write(fixture.root.join("note.md"), b"external").unwrap();
            assert!(matches!(
                broker.commit_recorded_restore(ticket),
                Err(BrokerError::RevisionConflict { .. })
            ));
        }
        assert!(!broker.content_audit_status().unwrap().restore_unconfirmed);
        let current = broker.read(&path()).unwrap();
        assert!(
            broker
                .mutate_recorded(
                    Mutation::replace(path(), b"next".to_vec(), current.revision, "next"),
                    super::metadata()
                )
                .is_ok()
        );
    }
}
