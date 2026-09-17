use super::*;
#[test]
fn delete_and_original_path_restore_have_distinct_sequences_and_exact_history() {
    let fixture = Fixture::new();
    let body = [0, 255, 1, 2, 128];
    let (mut broker, original) = fixture.seed(&body);
    let request = delete(original, "delete");
    let ticket = broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), body);
    assert_eq!(broker.write_status().unwrap().phase, WriterPhase::Prepared);
    intent(&mut broker, &ticket);
    let receipt = broker.commit_file_operation(&ticket).unwrap();
    assert_eq!(receipt.sequence, 1);
    assert!(!fixture.root.join("note.md").exists());
    assert!(broker.write_status().unwrap().read_blocked);
    assert!(
        broker
            .mutate(Mutation::create(path(), b"wrong".to_vec(), "other"))
            .is_err()
    );
    flush(&mut broker);
    let page = broker.list_tombstones(1, None).unwrap();
    assert_eq!(page.entries[0].revision, original);
    let history = broker.list_history(&path(), 100, None).unwrap();
    assert_eq!(history.entries[0].author, None);
    assert_eq!(
        broker.read_history(&path(), original, 32).unwrap().content,
        body
    );
    let restored = committed(&mut broker, restore(&page.entries[0].id, "restore"));
    assert_eq!(restored.sequence, 2);
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), body);
    assert_eq!(broker.list_tombstones(100, None).unwrap().entries.len(), 1);
    assert_eq!(
        broker.list_history(&path(), 100, None).unwrap().entries[0].author,
        Some(metadata().actor)
    );
    assert_eq!(
        broker
            .file_operation_replay(&request, &metadata())
            .unwrap()
            .unwrap()
            .receipt,
        Some(receipt)
    );
}
#[test]
fn intent_ack_is_required_and_cancelled_is_a_durable_original_result() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let request = delete(original, "cancel");
    let ticket = broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    assert!(matches!(
        broker.commit_file_operation(&ticket),
        Err(BrokerError::MutationUnresolved)
    ));
    let ticket = broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    assert!(broker.cancel_file_operation(ticket).is_err());
    let ticket = broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    intent(&mut broker, &ticket);
    assert_eq!(
        broker.cancel_file_operation(ticket).unwrap().outcome,
        FileOperationOutcome::Cancelled
    );
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
    assert!(broker.write_status().unwrap().unresolved);
    flush(&mut broker);
    drop(broker);
    let broker = fixture.reopen();
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
#[test]
fn initial_missing_rejection_cannot_become_success_after_aba() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    fs::remove_file(fixture.root.join("note.md")).unwrap();
    let request = delete(original, "reject");
    let ticket = broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    assert_eq!(
        ticket.preview().fixed_outcome,
        Some(FileOperationOutcome::Rejected)
    );
    fs::write(fixture.root.join("note.md"), b"old").unwrap();
    intent(&mut broker, &ticket);
    let receipt = broker.commit_file_operation(&ticket).unwrap();
    assert_eq!(receipt.outcome, FileOperationOutcome::Rejected);
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
    flush(&mut broker);
    assert_eq!(broker.write_status().unwrap().committed_through, 0);
}
#[test]
fn empty_is_present_and_restore_never_replaces_a_new_same_name_file() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"");
    let deleted = committed(&mut broker, delete(original, "empty"));
    fs::write(fixture.root.join("note.md"), b"new user content").unwrap();
    let ticket = broker
        .prepare_file_operation(
            restore(deleted.tombstone_id.as_deref().unwrap(), "restore"),
            metadata(),
        )
        .unwrap();
    assert_eq!(
        ticket.preview().fixed_outcome,
        Some(FileOperationOutcome::Rejected)
    );
    intent(&mut broker, &ticket);
    assert_eq!(
        broker.commit_file_operation(&ticket).unwrap().outcome,
        FileOperationOutcome::Rejected
    );
    assert_eq!(
        fs::read(fixture.root.join("note.md")).unwrap(),
        b"new user content"
    );
}
#[test]
fn completed_small_operations_release_unused_evidence_reservations() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    for index in 0..16 {
        let deleted = committed(&mut broker, delete(original, &format!("delete-{index}")));
        committed(
            &mut broker,
            restore(
                deleted.tombstone_id.as_deref().unwrap(),
                &format!("restore-{index}"),
            ),
        );
    }
    let (actual, _) = super::super::usage::actual(&broker).unwrap();
    assert!(actual < 1024 * 1024);
    assert!(!broker.write_status().unwrap().unresolved);
}
#[test]
fn newly_authorized_scope_can_restore_an_old_tombstone_without_rebinding_the_original_key() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let original_request = delete(original, "old-scope");
    let deleted = committed(&mut broker, original_request.clone());
    let mut current = metadata();
    current.scope_hash = FileOperationScopeHash::new("2".repeat(64)).unwrap();
    current.scope_epoch = 9;
    let ticket = broker
        .prepare_file_operation(
            restore(deleted.tombstone_id.as_deref().unwrap(), "new-scope"),
            current.clone(),
        )
        .unwrap();
    assert_eq!(ticket.intent().scope_epoch, 9);
    intent(&mut broker, &ticket);
    assert_eq!(
        broker.commit_file_operation(&ticket).unwrap().outcome,
        FileOperationOutcome::Committed
    );
    flush(&mut broker);
    assert!(matches!(
        broker.file_operation_replay(&original_request, &current),
        Err(BrokerError::IdempotencyConflict)
    ));
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
}
