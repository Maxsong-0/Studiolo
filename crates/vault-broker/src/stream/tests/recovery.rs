use super::*;
#[test]
fn every_persisted_stage_reopens_without_guessing_installation() {
    for point in [1, 2, 3, 4, 5] {
        let f = Fixture::new();
        let mut b = f.open();
        let source = f.bytes(b"payload");
        let request = request(&source, "pending", Precondition::MustNotExist);
        if point == 1 {
            fault(&b, point);
            assert!(
                b.prepare_stream_mutation(request.clone(), &source, limits(), metadata())
                    .is_err()
            );
        } else {
            let ticket = b
                .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
                .unwrap();
            fault(&b, point);
            assert!(b.commit_stream_mutation(ticket).is_err());
        }
        drop(b);
        let b = f.reopen();
        if point >= 4 {
            assert!(
                b.stream_mutation_replay(&request, &metadata())
                    .unwrap()
                    .is_some()
            );
        } else {
            assert!(
                b.stream_mutation_replay(&request, &metadata())
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                b.write_status().unwrap().phase,
                if point == 1 {
                    WriterPhase::Prepared
                } else {
                    WriterPhase::Installing
                }
            );
        }
    }
}
#[test]
fn large_installing_maintenance_preserves_three_objects_and_never_returns_original_success() {
    let _guard = LARGE.lock().unwrap();
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.pattern(5 * 1024 * 1024 + 1);
    let first = request(&source, "old", Precondition::MustNotExist);
    let ticket = b
        .prepare_stream_mutation(first.clone(), &source, limits(), metadata())
        .unwrap();
    let old = b.commit_stream_mutation(ticket).unwrap();
    ack(&mut b);
    // 第二个 proposed 追加少量字节，分别核对旧、新和外部当前三个独立对象。
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(f.source.join("input.bin"))
        .unwrap();
    file.write_all(b"new").unwrap();
    drop(file);
    let source = f.input();
    let request = request(&source, "unknown", Precondition::Match(old.revision));
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    fault(&b, 3);
    assert!(matches!(
        b.commit_stream_mutation(ticket),
        Err(BrokerError::MutationUnresolved)
    ));
    fs::write(f.root.join(path().as_str()), b"outside").unwrap();
    drop(b);
    let mut b = f.reopen();
    let target = b.maintenance_inspect(64).unwrap().targets.remove(0);
    assert!(target.proposed_available);
    let ticket = b
        .prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::PreserveInstalling,
            MaintenanceActor::current().unwrap(),
        )
        .unwrap();
    b.confirm_maintenance_intent(&ticket, 1, &"a".repeat(64))
        .unwrap();
    let receipt = b.commit_maintenance(ticket).unwrap();
    let id = receipt.event.evidence_id.unwrap();
    for (role, bytes) in [
        ("previous", first.content.bytes),
        ("proposed", request.content.bytes),
        ("current", 7),
    ] {
        assert_eq!(
            fs::metadata(
                f.data
                    .join("snapshots")
                    .join(format!("recovery-{id}-{role}.bin"))
            )
            .unwrap()
            .len(),
            bytes
        );
    }
    assert!(matches!(
        b.stream_mutation_replay(&request, &metadata()),
        Err(BrokerError::MutationIndeterminate { .. })
    ));
    assert_eq!(fs::read(f.root.join(path().as_str())).unwrap(), b"outside");
    assert!(b.write_status().unwrap().unresolved);
    ack(&mut b);
    for candidate in b.pending_maintenance_audit(64).unwrap() {
        b.confirm_maintenance_audit(candidate, 2, &"a".repeat(64))
            .unwrap();
    }
    assert!(!b.write_status().unwrap().unresolved);
}
#[test]
fn genuine_cas_rejection_is_terminal_even_if_missing_target_later_returns() {
    let f = Fixture::new();
    let mut b = f.open();
    let old = b
        .mutate(Mutation::create(path(), b"old".to_vec(), "old"))
        .unwrap();
    fs::remove_file(f.root.join(path().as_str())).unwrap();
    let source = f.bytes(b"new");
    let request = request(&source, "reject", Precondition::Match(old.revision));
    assert!(matches!(
        b.prepare_stream_mutation(request.clone(), &source, limits(), metadata()),
        Err(BrokerError::RevisionConflict { .. })
    ));
    fs::write(f.root.join(path().as_str()), b"old").unwrap();
    assert!(matches!(
        b.stream_mutation_replay(&request, &metadata()),
        Err(BrokerError::RevisionConflict { .. })
    ));
    assert_eq!(b.write_status().unwrap().committed_through, 1);
}
#[test]
fn real_post_rename_hardlink_failure_is_unresolved_and_does_not_replay_success() {
    let f = Fixture::new();
    let mut b = f.open();
    let source = f.bytes(b"installed");
    let request = request(&source, "unknown", Precondition::MustNotExist);
    let ticket = b
        .prepare_stream_mutation(request.clone(), &source, limits(), metadata())
        .unwrap();
    fault(&b, 17);
    assert!(matches!(
        b.commit_stream_mutation(ticket),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(
        fs::read(f.root.join(path().as_str())).unwrap(),
        b"installed"
    );
    assert_eq!(b.write_status().unwrap().phase, WriterPhase::Installing);
    assert!(
        b.stream_mutation_replay(&request, &metadata())
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        b.prepare_stream_mutation(request, &source, limits(), metadata()),
        Err(BrokerError::MutationUnresolved)
    ));
}
