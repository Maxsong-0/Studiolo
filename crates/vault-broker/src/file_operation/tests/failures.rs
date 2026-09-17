use super::*;
#[test]
fn issued_interruptions_never_reconstruct_success_from_current_hash_or_absence() {
    for point in [52, 54] {
        let fixture = Fixture::new();
        let (mut broker, original) = fixture.seed(b"old");
        let request = delete(original, "interrupted");
        let ticket = broker
            .prepare_file_operation(request.clone(), metadata())
            .unwrap();
        intent(&mut broker, &ticket);
        fault(&broker, point);
        assert!(matches!(
            broker.commit_file_operation(&ticket),
            Err(BrokerError::MutationUnresolved)
        ));
        assert_eq!(fixture.root.join("note.md").exists(), point == 52);
        drop(broker);
        let broker = fixture.reopen();
        let status = broker
            .file_operation_replay(&request, &metadata())
            .unwrap()
            .unwrap();
        assert_eq!(status.phase, FileOperationPhase::DetachIssued);
        assert!(status.receipt.is_none());
        assert_eq!(broker.write_status().unwrap().committed_through, 0);
    }
}
#[test]
fn known_detached_effect_can_only_finish_the_fixed_outcome_on_reopen() {
    for point in [55, 58, 59] {
        let fixture = Fixture::new();
        let (mut broker, original) = fixture.seed(b"old");
        let request = delete(original, "known");
        let ticket = broker
            .prepare_file_operation(request.clone(), metadata())
            .unwrap();
        intent(&mut broker, &ticket);
        if point == 59 {
            broker.commit_file_operation(&ticket).unwrap();
            fault(&broker, point);
            let candidate = broker.pending_file_operation_audit(64).unwrap().remove(0);
            assert!(
                broker
                    .confirm_file_operation_audit(candidate, 2, &"a".repeat(64))
                    .is_err()
            );
        } else {
            fault(&broker, point);
            assert!(broker.commit_file_operation(&ticket).is_err());
        }
        drop(broker);
        let mut broker = fixture.reopen();
        flush(&mut broker);
        assert_eq!(
            broker
                .file_operation_replay(&request, &metadata())
                .unwrap()
                .unwrap()
                .receipt
                .unwrap()
                .outcome,
            FileOperationOutcome::Committed
        );
        assert_eq!(broker.write_status().unwrap().committed_through, 1);
        assert!(!fixture.root.join("note.md").exists());
    }
}
#[test]
fn changed_l0_pin_or_source_aba_cannot_turn_old_authorization_into_new_delete() {
    for history in [false, true] {
        let fixture = Fixture::new();
        let (mut broker, original) = fixture.seed(b"old");
        let ticket = broker
            .prepare_file_operation(delete(original, "pin"), metadata())
            .unwrap();
        intent(&mut broker, &ticket);
        let target = if history {
            fixture.data.join("history").join(original.to_string())
        } else {
            fixture.root.join("note.md")
        };
        let replacement = target.with_extension("replacement");
        fs::write(&replacement, b"old").unwrap();
        fs::rename(replacement, target).unwrap();
        let result = broker.commit_file_operation(&ticket);
        if history {
            assert!(matches!(result, Err(BrokerError::CaptureChanged)));
        } else {
            assert!(matches!(result, Err(BrokerError::UnstableRead(_))));
        }
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
        assert_eq!(
            broker.file_operation_status().unwrap().unwrap().phase,
            FileOperationPhase::Prepared
        );
        // 最终复验失败仍保有同实例原票据，取消无需重新授权被替换的正文。
        assert_eq!(
            broker.cancel_file_operation(ticket).unwrap().outcome,
            FileOperationOutcome::Cancelled
        );
        assert!(broker.write_status().unwrap().unresolved);
        flush(&mut broker);
        assert!(!broker.write_status().unwrap().unresolved);
        assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
    }
}
#[test]
fn last_name_race_preserves_unexpected_detached_bytes_and_never_claims_commit() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let request = delete(original, "last-race");
    let ticket = broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    let id = ticket.preview().operation_id.clone();
    intent(&mut broker, &ticket);
    fault(&broker, 60);
    assert!(matches!(
        broker.commit_file_operation(&ticket),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(
        fs::read(
            fixture
                .root
                .join(".workbench/deleted-objects")
                .join(format!("{id}.object"))
        )
        .unwrap(),
        b"foreign"
    );
    assert_eq!(
        fs::read(fixture.data.join("history").join(original.to_string())).unwrap(),
        b"old"
    );
    assert!(
        broker
            .file_operation_replay(&request, &metadata())
            .unwrap()
            .unwrap()
            .receipt
            .is_none()
    );
}
#[test]
fn same_key_cannot_cross_legacy_or_typed_preparing_without_a_terminal() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let request = delete(original, "held");
    fault(&broker, 50);
    assert!(broker.prepare_file_operation(request, metadata()).is_err());
    let old = Mutation::replace(path(), b"next".to_vec(), original, "held");
    assert!(matches!(
        broker.mutation_replay(&old),
        Err(BrokerError::IdempotencyConflict)
    ));
    assert!(matches!(
        broker.mutate(old),
        Err(BrokerError::IdempotencyConflict)
    ));
    drop(broker);
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    let initial = broker
        .mutate(Mutation::create(path(), b"old".to_vec(), "legacy"))
        .unwrap();
    broker.enable_file_operations(limits()).unwrap();
    assert!(matches!(
        broker.prepare_file_operation(delete(initial.revision, "legacy"), metadata()),
        Err(BrokerError::IdempotencyConflict)
    ));
}
#[test]
fn restore_issued_after_real_install_stays_unknown_until_explicit_preservation() {
    let fixture = Fixture::new();
    let (mut broker, original) = fixture.seed(b"old");
    let deleted = committed(&mut broker, delete(original, "remove"));
    let request = restore(deleted.tombstone_id.as_deref().unwrap(), "restore-unknown");
    let ticket = broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    intent(&mut broker, &ticket);
    fault(&broker, 54);
    assert!(matches!(
        broker.commit_file_operation(&ticket),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(fs::read(fixture.root.join("note.md")).unwrap(), b"old");
    drop(broker);
    let broker = fixture.reopen();
    let pending = broker
        .file_operation_replay(&request, &metadata())
        .unwrap()
        .unwrap();
    assert_eq!(pending.phase, FileOperationPhase::InstallIssued);
    assert!(pending.receipt.is_none());
    assert_eq!(broker.write_status().unwrap().committed_through, 1);
}
