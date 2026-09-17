use super::*;

#[test]
fn legacy_success_receipts_supply_unknown_history_without_rewriting_original_files() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let previous = Revision::compute(broker.vault_uuid(), &path(), b"A");
    let current = Revision::compute(broker.vault_uuid(), &path(), b"B");
    fs::write(fixture.root.join("note.md"), b"B").unwrap();
    fs::write(fixture.data.join("history").join(previous.to_hex()), b"A").unwrap();
    let old_request = Mutation::replace(path(), b"B".to_vec(), previous, "legacy");
    let old = MutationReplay {
        fingerprint: FsVaultBroker::fingerprint(&old_request).unwrap(),
        expected_revision: current,
        receipt: Some(MutationReceipt {
            path: path(),
            revision: current,
            previous_revision: Some(previous),
        }),
    };
    let bytes = serde_json::to_vec(&old).unwrap();
    let filename = fixture
        .data
        .join("idempotency")
        .join(format!("{}.json", sha256(b"legacy")));
    fs::write(&filename, &bytes).unwrap();
    let page = broker.list_history(&path(), 10, None).unwrap();
    assert_eq!(page.entries.len(), 1);
    let entry = &page.entries[0];
    assert!(entry.authored_at_ms.is_none() && entry.archived_at_ms.is_none());
    assert!(entry.author.is_none() && entry.archived_by.is_none());
    assert_eq!(
        broker.read_history(&path(), previous, 10).unwrap().content,
        b"A"
    );
    assert!(!fixture.data.join("history/meta-v1").exists());
    broker
        .mutate_recorded(
            Mutation::replace(path(), b"C".to_vec(), current, "new"),
            metadata(),
        )
        .unwrap();
    assert_eq!(fs::read(filename).unwrap(), bytes);
    assert_eq!(
        broker
            .list_history(&path(), 10, None)
            .unwrap()
            .entries
            .len(),
        3
    );
}

#[test]
fn version_one_prepared_operation_remains_resumable_without_invented_metadata() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    interrupt(&broker, 1);
    assert!(broker.mutate(request(b"A", "first")).is_err());
    let mut state = crate::mutation::load(&broker).unwrap();
    state.format_version = 1;
    state.operation.as_mut().unwrap().recording = None;
    fs::write(
        fixture.data.join("writer-state.json"),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    drop(broker);
    let mut broker = fixture.reopen();
    assert!(matches!(
        broker.mutate_recorded(request(b"A", "first"), metadata()),
        Err(BrokerError::IdempotencyConflict | BrokerError::MutationUnresolved)
    ));
    broker.mutate(request(b"A", "first")).unwrap();
    let page = broker.list_history(&path(), 10, None).unwrap();
    assert_eq!(page.entries.len(), 1);
    assert!(page.entries[0].author.is_none() && page.entries[0].authored_at_ms.is_none());
}

#[test]
fn unknown_schema_or_half_initialized_schema_is_not_overwritten() {
    for partial in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        if partial {
            fs::create_dir(fixture.data.join("history/meta-v1.preparing")).unwrap();
        } else {
            broker
                .mutate_recorded(request(b"A", "first"), metadata())
                .unwrap();
            let filename = fixture.data.join("history/meta-v1/binding.json");
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&filename).unwrap()).unwrap();
            value["version"] = 9.into();
            fs::write(filename, serde_json::to_vec(&value).unwrap()).unwrap();
        }
        assert!(
            broker
                .mutate_recorded(
                    Mutation::create(
                        VaultPath::parse("other.md").unwrap(),
                        b"B".to_vec(),
                        "second"
                    ),
                    metadata()
                )
                .is_err()
        );
        assert!(!fixture.root.join("other.md").exists());
    }
}

#[test]
fn restored_terminal_remains_replayable_without_original_history_bytes() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (source, expected) = versions(&mut broker);
    let (request, metadata) = restore_request(source, expected, "restore");
    let ticket = broker.prepare_recorded_restore(request, metadata).unwrap();
    broker
        .confirm_recorded_restore_intent(&ticket, 1, &"a".repeat(64))
        .unwrap();
    let receipt = broker.commit_recorded_restore(ticket).unwrap();
    fs::remove_file(fixture.data.join("history").join(source.to_hex())).unwrap();
    fs::write(fixture.root.join("note.md"), b"later").unwrap();
    assert_eq!(
        broker
            .restore_replay(&path(), expected, source, "restore", &actor())
            .unwrap(),
        Some(receipt)
    );
    assert!(matches!(
        broker.restore_replay(
            &VaultPath::parse("other.md").unwrap(),
            expected,
            source,
            "restore",
            &actor()
        ),
        Err(BrokerError::IdempotencyConflict)
    ));
    assert!(matches!(
        broker.restore_replay(
            &path(),
            expected,
            source,
            "restore",
            &MutationActor::Obsidian
        ),
        Err(BrokerError::IdempotencyConflict)
    ));
    assert_eq!(fixture.note(), b"later");
}

#[test]
fn recorded_replay_cannot_change_restore_kind_or_actor() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let (source, expected) = versions(&mut broker);
    let (request, metadata) = restore_request(source, expected, "restore");
    let ticket = broker
        .prepare_recorded_restore(request.clone(), metadata.clone())
        .unwrap();
    broker
        .confirm_recorded_restore_intent(&ticket, 1, &"a".repeat(64))
        .unwrap();
    let receipt = broker.commit_recorded_restore(ticket).unwrap();
    assert!(broker.content_audit_status().unwrap().restore_unconfirmed);
    assert!(matches!(
        broker.mutation_replay_recorded(&request, &super::metadata()),
        Err(BrokerError::IdempotencyConflict)
    ));
    let other = MutationMetadata::new(MutationActor::Obsidian, metadata.kind.clone()).unwrap();
    assert!(matches!(
        broker.mutation_replay_recorded(&request, &other),
        Err(BrokerError::IdempotencyConflict)
    ));
    assert_eq!(
        broker
            .mutation_replay_recorded(&request, &metadata)
            .unwrap(),
        Some(receipt)
    );
}
