use super::*;

#[test]
fn ordinary_save_succeeds_without_audit_and_delivers_fixed_intent_then_outcome() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let receipt = broker
        .mutate_recorded(request(b"private content", "first"), metadata())
        .unwrap();
    assert_eq!(fixture.note(), b"private content");
    assert_eq!(
        broker.content_audit_status().unwrap(),
        ContentAuditStatus {
            pending: 2,
            restore_unconfirmed: false
        }
    );
    let candidate = broker.pending_content_audit(64).unwrap().remove(0);
    assert_eq!(candidate.event().phase, ContentAuditPhase::Intent);
    let serialized = serde_json::to_string(candidate.event()).unwrap();
    assert!(!serialized.contains("private content"));
    assert_eq!(candidate.event().new_revision, receipt.revision);
    broker
        .confirm_content_audit(candidate, 1, &"a".repeat(64))
        .unwrap();
    assert_eq!(broker.content_audit_status().unwrap().pending, 1);
    let candidate = broker.pending_content_audit(64).unwrap().remove(0);
    assert_eq!(
        candidate.event().outcome,
        Some(ContentAuditOutcome::Committed)
    );
    broker
        .confirm_content_audit(candidate, 2, &"a".repeat(64))
        .unwrap();
    assert!(broker.pending_content_audit(64).unwrap().is_empty());
}

#[test]
fn preflight_rejection_is_durable_auditable_and_not_resurrected_after_aba() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let first = broker.mutate(request(b"A", "initial")).unwrap();
    let attempted = Mutation::replace(path(), b"C".to_vec(), first.revision, "rejected");
    fs::remove_file(fixture.root.join("note.md")).unwrap();
    interrupt(&broker, 6);
    assert!(
        broker
            .check_precondition_recorded(&attempted, metadata())
            .is_err()
    );
    drop(broker);
    fs::write(fixture.root.join("note.md"), b"A").unwrap();
    let mut broker = fixture.reopen();
    assert!(matches!(
        broker.mutate_recorded(attempted, metadata()),
        Err(BrokerError::RevisionConflict { actual: None, .. })
    ));
    assert_eq!(broker.content_audit_status().unwrap().pending, 2);
    let intent = broker.pending_content_audit(64).unwrap().remove(0);
    broker
        .confirm_content_audit(intent, 1, &"a".repeat(64))
        .unwrap();
    assert_eq!(
        broker.pending_content_audit(64).unwrap()[0].event().outcome,
        Some(ContentAuditOutcome::Rejected)
    );
}

#[test]
fn completed_and_index_interruptions_reopen_without_reinstalling_or_losing_events() {
    for point in [4, 5, 14] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        interrupt(&broker, point);
        assert!(
            broker
                .mutate_recorded(request(b"A", "first"), metadata())
                .is_err()
        );
        fs::write(fixture.root.join("note.md"), b"external").unwrap();
        drop(broker);
        let mut broker = fixture.reopen();
        assert_eq!(broker.write_status().unwrap().committed_through, 1);
        assert_eq!(
            broker
                .list_history(&path(), 10, None)
                .unwrap()
                .entries
                .len(),
            1
        );
        assert_eq!(broker.content_audit_status().unwrap().pending, 2);
        assert!(
            broker
                .mutate_recorded(request(b"A", "first"), metadata())
                .is_ok()
        );
        assert_eq!(fixture.note(), b"external");
        ack(&mut broker);
    }
}

#[test]
fn ack_interruption_replays_same_event_and_only_outcome_ack_cleans_marker() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    broker
        .mutate_recorded(request(b"A", "first"), metadata())
        .unwrap();
    let intent = broker.pending_content_audit(64).unwrap().remove(0);
    interrupt(&broker, 13);
    assert!(
        broker
            .confirm_content_audit(intent, 1, &"a".repeat(64))
            .is_err()
    );
    drop(broker);
    let mut broker = fixture.reopen();
    let outcome = broker.pending_content_audit(64).unwrap().remove(0);
    assert_eq!(outcome.event().phase, ContentAuditPhase::Outcome);
    interrupt(&broker, 13);
    assert!(
        broker
            .confirm_content_audit(outcome, 2, &"a".repeat(64))
            .is_err()
    );
    drop(broker);
    let mut broker = fixture.reopen();
    assert_eq!(broker.content_audit_status().unwrap().pending, 0);
    assert_eq!(broker.pending_content_audit(64).unwrap().len(), 1);
    ack(&mut broker);
}

#[test]
fn candidate_and_actor_are_bound_and_old_instance_cannot_ack() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    broker
        .mutate_recorded(request(b"A", "first"), metadata())
        .unwrap();
    let other = MutationMetadata::new(MutationActor::Obsidian, MutationKind::Replace).unwrap();
    assert!(matches!(
        broker.mutate_recorded(request(b"A", "first"), other),
        Err(BrokerError::IdempotencyConflict)
    ));
    let candidate = broker.pending_content_audit(64).unwrap().remove(0);
    drop(broker);
    let mut broker = fixture.reopen();
    assert!(matches!(
        broker.confirm_content_audit(candidate, 1, &"a".repeat(64)),
        Err(BrokerError::CaptureChanged)
    ));
    let candidate = broker.pending_content_audit(64).unwrap().remove(0);
    assert!(
        broker
            .confirm_content_audit(candidate, 0, &"a".repeat(64))
            .is_err()
    );
    assert_eq!(broker.content_audit_status().unwrap().pending, 2);
}

#[test]
fn corrupt_outbox_before_save_prevents_installation() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    broker
        .mutate_recorded(request(b"A", "first"), metadata())
        .unwrap();
    fs::write(
        fixture.data.join("history/meta-v1/outbox/unexpected"),
        b"bad",
    )
    .unwrap();
    let attempt = Mutation::replace(
        path(),
        b"B".to_vec(),
        broker.read(&path()).unwrap().revision,
        "second",
    );
    assert!(broker.mutate_recorded(attempt, metadata()).is_err());
    assert_eq!(fixture.note(), b"A");
}
