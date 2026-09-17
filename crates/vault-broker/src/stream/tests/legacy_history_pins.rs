use super::history_pins::damage;
use super::*;

#[test]
fn legacy_vec_rechecks_l0_before_installing_and_at_final_rename() {
    for point in [80, 81, 82, 83] {
        let f = Fixture::new();
        let mut b = f.open();
        let old = b
            .mutate(Mutation::create(path(), b"old".to_vec(), "old"))
            .unwrap();
        let request = Mutation::replace(path(), b"new".to_vec(), old.revision, "next");
        fault(&b, point);
        let result = b.mutate(request.clone());
        assert!(result.is_err());
        let phase = if point < 82 {
            WriterPhase::Prepared
        } else {
            WriterPhase::Installing
        };
        if point >= 82 {
            assert!(matches!(result, Err(BrokerError::MutationUnresolved)));
        }
        assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"old");
        assert_eq!(b.write_status().unwrap().phase, phase);
        assert_eq!(b.write_status().unwrap().committed_through, 1);
        assert!(b.mutation_replay(&request).unwrap().is_none());
        drop(b);
        let b = f.reopen();
        assert_eq!(b.write_status().unwrap().phase, phase);
        assert!(b.mutation_replay(&request).unwrap().is_none());
    }
}

#[test]
fn legacy_prepared_replay_refuses_corrupt_old_or_new_l0() {
    for previous in [false, true] {
        let f = Fixture::new();
        let mut b = f.open();
        let old = b
            .mutate(Mutation::create(path(), b"old".to_vec(), "old"))
            .unwrap();
        let request = Mutation::replace(path(), b"new".to_vec(), old.revision, "next");
        fault(&b, 1);
        assert!(b.mutate(request.clone()).is_err());
        let revision = if previous {
            old.revision
        } else {
            Revision::compute(b.vault_uuid(), &path(), b"new")
        };
        damage(&f, revision, 1);
        drop(b);
        let mut b = f.reopen();
        assert!(b.mutate(request.clone()).is_err());
        assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"old");
        assert_eq!(b.write_status().unwrap().phase, WriterPhase::Prepared);
        assert!(b.mutation_replay(&request).unwrap().is_none());
    }
}

#[test]
fn legacy_prepared_retry_can_represerve_missing_l0_only_from_exact_original_bytes() {
    let f = Fixture::new();
    let mut b = f.open();
    let old = b
        .mutate(Mutation::create(path(), b"old".to_vec(), "old"))
        .unwrap();
    let request = Mutation::replace(path(), b"new".to_vec(), old.revision, "next");
    fault(&b, 1);
    assert!(b.mutate(request.clone()).is_err());
    damage(&f, old.revision, 0);
    drop(b);
    let mut b = f.reopen();
    let receipt = b.mutate(request.clone()).unwrap();
    assert_eq!(
        fs::read(f.data.join("history").join(old.revision.to_string())).unwrap(),
        b"old"
    );
    assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"new");
    assert_eq!(b.mutation_replay(&request).unwrap(), Some(receipt));
}

#[test]
fn legacy_vec_keeps_large_previous_canonical_when_its_l0_drifts_after_installing() {
    let _guard = LARGE.lock().unwrap();
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.pattern(5 * 1024 * 1024 + 1);
    let ticket = b
        .prepare_stream_mutation(
            request(&source, "large-old", Precondition::MustNotExist),
            &source,
            limits(),
            metadata(),
        )
        .unwrap();
    let old = b.commit_stream_mutation(ticket).unwrap();
    let request = Mutation::replace(path(), b"new".to_vec(), old.revision, "small");
    fault(&b, 82);
    assert!(matches!(
        b.mutate(request.clone()),
        Err(BrokerError::MutationUnresolved)
    ));
    let current = installed(&b);
    assert_eq!(current.revision, Some(old.revision));
    assert_eq!(current.identity.bytes, 5 * 1024 * 1024 + 1);
    assert_eq!(b.write_status().unwrap().phase, WriterPhase::Installing);
    assert!(b.mutation_replay(&request).unwrap().is_none());
}
