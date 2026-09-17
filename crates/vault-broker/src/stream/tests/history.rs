use super::*;
#[test]
fn strict_restore_cannot_commit_or_cancel_before_its_fixed_intent_ack() {
    for cancel in [false, true] {
        let f = Fixture::new();
        let mut b = f.open();
        let old = b
            .mutate(Mutation::create(path(), b"old".to_vec(), "old"))
            .unwrap();
        let current = b
            .mutate(Mutation::replace(
                path(),
                b"new".to_vec(),
                old.revision,
                "new",
            ))
            .unwrap();
        let ticket = b
            .prepare_stream_restore(
                path(),
                current.revision,
                old.revision,
                "restore".into(),
                actor(),
                limits(),
            )
            .unwrap();
        let result = if cancel {
            b.cancel_stream_mutation(ticket)
        } else {
            b.commit_stream_mutation(ticket).map(|_| ())
        };
        assert!(matches!(result, Err(BrokerError::MutationUnresolved)));
        assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"new");
        assert_eq!(b.write_status().unwrap().phase, WriterPhase::Prepared);
    }
}
#[test]
fn old_fingerprints_remain_byte_for_byte_after_stream_and_legacy_writes() {
    let f = Fixture::new();
    let mut b = f.open();
    let old_request = Mutation::create(path(), b"old".to_vec(), "old");
    let old = b.mutate(old_request.clone()).unwrap();
    let receipt_file = f
        .data
        .join("idempotency")
        .join(format!("{}.json", crate::sha256(b"old")));
    let original = fs::read(&receipt_file).unwrap();
    let source = f.bytes(b"stream");
    let r = request(&source, "stream", Precondition::Match(old.revision));
    let ticket = b
        .prepare_stream_mutation(r, &source, limits(), metadata())
        .unwrap();
    let value = b.commit_stream_mutation(ticket).unwrap();
    b.mutate(Mutation::replace(
        path(),
        b"again".to_vec(),
        value.revision,
        "legacy-again",
    ))
    .unwrap();
    assert_eq!(fs::read(receipt_file).unwrap(), original);
    assert_eq!(b.mutation_replay(&old_request).unwrap(), Some(old));
    assert_eq!(mutation::load(&b).unwrap().format_version, 4);
    drop(b);
    assert_eq!(mutation::load(&f.reopen()).unwrap().format_version, 4);
}
#[test]
fn effective_limit_downshift_does_not_reduce_old_object_preservation_limit() {
    let _guard = LARGE.lock().unwrap();
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.pattern(5 * 1024 * 1024 + 1);
    let r = request(&source, "large", Precondition::MustNotExist);
    let ticket = b
        .prepare_stream_mutation(r.clone(), &source, limits(), metadata())
        .unwrap();
    let old = b.commit_stream_mutation(ticket).unwrap();
    let small = f.bytes(b"x");
    let new = request(&small, "new", Precondition::Match(old.revision));
    let ticket = b
        .prepare_stream_mutation(new, &small, StreamLimits::new(1).unwrap(), metadata())
        .unwrap();
    b.commit_stream_mutation(ticket).unwrap();
    assert_eq!(
        crate::history::blob::selected(&b, &path(), old.revision).unwrap(),
        r.content
    );
}
#[test]
fn first_legacy_write_over_an_external_large_file_activates_large_history_before_install() {
    let _guard = LARGE.lock().unwrap();
    let f = Fixture::new();
    let mut b = f.open();
    f.pattern(5 * 1024 * 1024 + 1);
    fs::create_dir_all(f.root.join("attachments")).unwrap();
    fs::copy(f.source.join("input.bin"), f.root.join(path().as_str())).unwrap();
    let old = installed(&b).revision.unwrap();
    b.mutate(Mutation::replace(path(), b"small".to_vec(), old, "small"))
        .unwrap();
    assert_eq!(mutation::load(&b).unwrap().format_version, 4);
    assert_eq!(
        crate::history::blob::selected(&b, &path(), old)
            .unwrap()
            .bytes,
        5 * 1024 * 1024 + 1
    );
}
