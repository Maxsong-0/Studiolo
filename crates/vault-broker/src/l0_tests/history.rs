use super::*;

#[test]
fn saved_history_keeps_real_authorship_and_marks_external_old_bytes_unknown() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    fs::write(fixture.root.join("note.md"), b"outside").unwrap();
    let old = broker.read(&path()).unwrap().revision;
    let new = broker
        .mutate_recorded(
            Mutation::replace(path(), b"saved".to_vec(), old, "saved"),
            metadata(),
        )
        .unwrap()
        .revision;
    let page = broker.list_history(&path(), 10, None).unwrap();
    let old = page.entries.iter().find(|e| e.revision == old).unwrap();
    assert!(old.author.is_none() && old.authored_at_ms.is_none());
    assert_eq!(old.archived_by, Some(actor()));
    let new = page.entries.iter().find(|e| e.revision == new).unwrap();
    assert_eq!(new.author, Some(actor()));
    assert!(new.authored_at_ms.is_some() && new.archived_at_ms.is_some());
    assert_eq!(
        broker
            .read_history(&path(), old.revision, 1024)
            .unwrap()
            .content,
        b"outside"
    );
}

#[test]
fn cursor_distinguishes_syntax_wrong_path_and_changed_generation() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    versions(&mut broker);
    let first = broker.list_history(&path(), 1, None).unwrap();
    let cursor = first.next_cursor.unwrap();
    let second = broker.list_history(&path(), 1, Some(&cursor)).unwrap();
    assert_ne!(first.entries[0].id, second.entries[0].id);
    assert!(second.next_cursor.is_none());
    assert!(matches!(
        broker.list_history(&path(), 1, Some("bad")),
        Err(BrokerError::InvalidHistoryCursor)
    ));
    assert!(matches!(
        broker.list_history(&VaultPath::parse("other.md").unwrap(), 1, Some(&cursor)),
        Err(BrokerError::InvalidHistoryCursor)
    ));
    let previous = broker.read(&path()).unwrap().revision;
    broker
        .mutate_recorded(
            Mutation::replace(path(), b"C".to_vec(), previous, "third"),
            metadata(),
        )
        .unwrap();
    assert!(matches!(
        broker.list_history(&path(), 1, Some(&cursor)),
        Err(BrokerError::HistoryChanged)
    ));
}

#[test]
fn history_checks_bytes_path_uuid_and_limits_and_ignores_canonical_later_changes() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let receipt = broker
        .mutate_recorded(request(b"content", "first"), metadata())
        .unwrap();
    fs::remove_file(fixture.root.join("note.md")).unwrap();
    assert_eq!(
        broker
            .read_history(&path(), receipt.revision, 100)
            .unwrap()
            .content,
        b"content"
    );
    assert!(matches!(
        broker.read_history(&path(), receipt.revision, 1),
        Err(BrokerError::ReadLimitExceeded { max_bytes: 1, .. })
    ));
    assert!(matches!(
        broker.read_history(
            &VaultPath::parse("other.md").unwrap(),
            receipt.revision,
            100
        ),
        Err(BrokerError::NotFound(_))
    ));
    fs::write(
        fixture.data.join("history").join(receipt.revision.to_hex()),
        b"changed",
    )
    .unwrap();
    assert!(matches!(
        broker.read_history(&path(), receipt.revision, 100),
        Err(BrokerError::CorruptWriterState)
    ));
}

#[test]
fn history_root_and_metadata_binding_changes_fail_closed() {
    for metadata in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        let receipt = broker
            .mutate_recorded(request(b"A", "first"), super::metadata())
            .unwrap();
        if metadata {
            let original = fixture.data.join("history/meta-v1/catalogs");
            fs::rename(&original, fixture.data.join("history/meta-v1/old-catalogs")).unwrap();
            fs::create_dir(&original).unwrap();
        } else {
            fs::rename(&fixture.root, fixture._temp.path().join("old-root")).unwrap();
            fs::create_dir(&fixture.root).unwrap();
        }
        assert!(broker.list_history(&path(), 10, None).is_err());
        assert!(broker.read_history(&path(), receipt.revision, 10).is_err());
    }
}

#[test]
fn prepared_versions_are_not_exposed_as_successful_history() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let receipt = broker
        .mutate_recorded(request(b"A", "first"), metadata())
        .unwrap();
    let attempt = Mutation::replace(path(), b"B".to_vec(), receipt.revision, "second");
    let target = Revision::compute(broker.vault_uuid(), &path(), b"B");
    interrupt(&broker, 1);
    assert!(broker.mutate_recorded(attempt, metadata()).is_err());
    assert_eq!(
        broker
            .list_history(&path(), 10, None)
            .unwrap()
            .entries
            .len(),
        1
    );
    assert!(matches!(
        broker.read_history(&path(), target, 100),
        Err(BrokerError::NotFound(_))
    ));
    let review = broker.pending_mutation().unwrap().unwrap();
    broker
        .reconcile_pending(&review, PendingDecision::CancelPrepared)
        .unwrap();
    assert_eq!(
        broker
            .list_history(&path(), 10, None)
            .unwrap()
            .entries
            .len(),
        1
    );
}
