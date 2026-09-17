use super::*;
#[test]
fn broken_required_history_fails_before_intent_prepared_and_installation() {
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.bytes(b"new");
    let revision = Revision::compute(b.vault_uuid, &path(), b"new");
    fs::write(f.data.join("history").join(revision.to_string()), b"broken").unwrap();
    assert!(
        b.prepare_stream_mutation(
            request(&source, "first", Precondition::MustNotExist),
            &source,
            limits(),
            metadata()
        )
        .is_err()
    );
    assert!(!f.root.join(path().as_str()).exists());
    assert_eq!(b.write_status().unwrap().phase, WriterPhase::Ready);
    assert!(b.pending_content_audit(64).unwrap().is_empty());
}
#[test]
fn new_descriptor_cannot_be_silently_loaded_under_old_writer_semantics() {
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.bytes(b"small");
    let ticket = b
        .prepare_stream_mutation(
            request(&source, "first", Precondition::MustNotExist),
            &source,
            limits(),
            metadata(),
        )
        .unwrap();
    drop(ticket);
    drop(b);
    let filename = f.data.join("writer-state.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&filename).unwrap()).unwrap();
    value["format_version"] = 2.into();
    fs::write(filename, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        FsVaultBroker::open_existing(&f.root, &f.data),
        Err(BrokerError::CorruptWriterState)
    ));
}
