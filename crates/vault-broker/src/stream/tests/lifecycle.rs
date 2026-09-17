use super::*;
#[test]
fn prepare_is_not_install_and_one_slot_refuses_another_request() {
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.bytes(b"hello");
    let request = request(&source, "first", Precondition::MustNotExist);
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    assert!(!f.root.join(path().as_str()).exists());
    assert_eq!(b.write_status().unwrap().phase, WriterPhase::Prepared);
    let other = request.clone();
    assert!(matches!(
        b.prepare_stream_mutation(other, &source, limits(), metadata()),
        Err(BrokerError::MutationUnresolved)
    ));
    let receipt = b.commit_stream_mutation(ticket).unwrap();
    assert_eq!(
        b.stream_mutation_replay(&request, &metadata()).unwrap(),
        Some(receipt)
    );
    assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"hello");
}
#[test]
fn cancelled_ticket_has_permanent_original_key_outcome() {
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.bytes(b"hello");
    let request = request(&source, "first", Precondition::MustNotExist);
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    b.cancel_stream_mutation(ticket).unwrap();
    assert!(matches!(
        b.stream_mutation_replay(&request, &metadata()),
        Err(BrokerError::MutationCancelled)
    ));
    assert!(!f.root.join(path().as_str()).exists());
    assert!(
        b.pending_content_audit(64)
            .unwrap()
            .iter()
            .all(|candidate| candidate.event().outcome != Some(ContentAuditOutcome::Committed))
    );
    ack(&mut b);
    assert_eq!(b.content_audit_status().unwrap().pending, 0);
}
