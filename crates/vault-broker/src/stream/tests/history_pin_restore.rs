use super::history_pins::damage;
use super::*;

#[test]
fn same_revision_preservation_has_one_proof_and_commits_normally() {
    let f = Fixture::new();
    let mut b = f.open();
    let old = b
        .mutate(Mutation::create(path(), b"same".to_vec(), "old"))
        .unwrap();
    let source = f.bytes(b"same");
    let request = request(&source, "same", Precondition::Match(old.revision));
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    assert_eq!(ticket.target_revision(), old.revision);
    assert_eq!(ticket.history.object_count(), 1);
    let receipt = b.commit_stream_mutation(ticket).unwrap();
    assert_eq!(receipt.revision, old.revision);
    assert_eq!(
        b.stream_mutation_replay(&request, &metadata()).unwrap(),
        Some(receipt)
    );
    assert_eq!(b.write_status().unwrap().committed_through, 2);
}

#[test]
fn acknowledged_restore_rechecks_both_l0_objects_before_installation() {
    for previous in [false, true] {
        for kind in 0..3 {
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
            b.confirm_stream_restore_intent(&ticket, 1, &"a".repeat(64))
                .unwrap();
            damage(
                &f,
                if previous {
                    current.revision
                } else {
                    old.revision
                },
                kind,
            );
            assert!(b.commit_stream_mutation(ticket).is_err());
            assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"new");
            assert_eq!(b.write_status().unwrap().phase, WriterPhase::Prepared);
            assert!(
                b.restore_replay(&path(), current.revision, old.revision, "restore", &actor())
                    .unwrap()
                    .is_none()
            );
        }
    }
}

#[test]
fn l0_drift_does_not_prevent_explicit_prepared_cancel_and_a_new_safe_attempt() {
    let f = Fixture::new();
    let mut b = f.open();
    let old = b
        .mutate(Mutation::create(path(), b"old".to_vec(), "old"))
        .unwrap();
    let source = f.bytes(b"new");
    let request = request(&source, "cancel", Precondition::Match(old.revision));
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    damage(&f, old.revision, 0);
    b.cancel_stream_mutation(ticket).unwrap();
    assert!(matches!(
        b.stream_mutation_replay(&request, &metadata()),
        Err(BrokerError::MutationCancelled)
    ));
    assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"old");
    let mut next = request;
    next.idempotency_key = "fresh-after-cancel".into();
    let ticket = b
        .prepare_stream_mutation(next, &source, limits(), metadata())
        .unwrap();
    b.commit_stream_mutation(ticket).unwrap();
    assert_eq!(
        fs::read(f.data.join("history").join(old.revision.to_string())).unwrap(),
        b"old"
    );
    assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"new");
}
