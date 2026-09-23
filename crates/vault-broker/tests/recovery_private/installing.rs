use super::*;

#[test]
fn unknown_installations_keep_current_and_all_three_versions_without_success() {
    for point in [2, 3] {
        let f = Fixture::new();
        let (mut broker, request) = f.interrupted(point);
        fs::write(f.root.join("note.md"), b"external").unwrap();
        let review = broker.pending_mutation().unwrap().unwrap();
        let receipt = broker.reconcile_pending(&review, preserve()).unwrap();
        assert_eq!(receipt.outcome, ReconciliationOutcome::Indeterminate);
        assert_eq!(receipt.committed_through, 1);
        let id = receipt.evidence_id.unwrap();
        assert_eq!(f.evidence(&id, "proposed"), b"proposed");
        assert_eq!(f.evidence(&id, "previous"), b"old");
        assert_eq!(f.evidence(&id, "current"), b"external");
        assert_eq!(broker.read(&path()).unwrap().content, b"external");
        assert_ready(&broker);
        assert!(
            matches!(broker.mutate(request.clone()), Err(BrokerError::MutationIndeterminate { evidence_id }) if evidence_id == id)
        );
        assert!(matches!(
            broker.mutation_replay(&request),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
        drop(broker);
        let mut reopened = f.open();
        assert!(matches!(
            reopened.mutate(request),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
        assert_eq!(reopened.prune_stages().unwrap().removed, 0);
        assert_eq!(f.evidence(&id, "proposed"), b"proposed");
    }
}

#[test]
fn same_content_and_original_installed_inode_are_never_promoted_to_success() {
    for point in [2, 3] {
        let f = Fixture::new();
        let (mut broker, request) = f.interrupted(point);
        if point == 2 {
            fs::write(f.root.join("note.md"), b"proposed").unwrap();
        }
        let review = broker.pending_mutation().unwrap().unwrap();
        assert_eq!(review.current_revision(), Some(review.target_revision()));
        broker.reconcile_pending(&review, preserve()).unwrap();
        assert!(matches!(
            broker.mutate(request),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
        assert_eq!(broker.write_status().unwrap().committed_through, 1);
    }
}

#[test]
fn missing_current_is_preserved_as_absence_not_an_empty_file() {
    let f = Fixture::new();
    let (mut broker, _) = f.interrupted(2);
    fs::remove_file(f.root.join("note.md")).unwrap();
    let review = broker.pending_mutation().unwrap().unwrap();
    let receipt = broker.reconcile_pending(&review, preserve()).unwrap();
    assert!(receipt.current_revision.is_none());
    assert!(!f.root.join("note.md").exists());
    let id = receipt.evidence_id.unwrap();
    assert!(
        !f.data
            .join("snapshots")
            .join(format!("recovery-{id}-current.bin"))
            .exists()
    );
    assert_eq!(f.evidence(&id, "previous"), b"old");
}

#[test]
fn interruption_before_terminal_stays_pending_but_after_terminal_only_finishes_cleanup() {
    for point in [7, 8, 9] {
        let f = Fixture::new();
        let (mut broker, request) = f.interrupted(2);
        let review = broker.pending_mutation().unwrap().unwrap();
        interrupt(&broker, point);
        assert!(matches!(
            broker.reconcile_pending(&review, preserve()),
            Err(BrokerError::Io {
                operation: "injected writer interruption",
                ..
            })
        ));
        drop(broker);
        let mut reopened = f.open();
        if point == 7 {
            assert!(reopened.write_status().unwrap().read_blocked);
            let next = reopened.pending_mutation().unwrap().unwrap();
            reopened.reconcile_pending(&next, preserve()).unwrap();
        }
        assert_ready(&reopened);
        assert!(matches!(
            reopened.mutate(request),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
    }
}

#[test]
fn corrupted_or_missing_sealed_evidence_cannot_finish_unlocking_on_reopen() {
    for damage in ["bytes", "missing"] {
        let f = Fixture::new();
        let (mut broker, _) = f.interrupted(2);
        let review = broker.pending_mutation().unwrap().unwrap();
        interrupt(&broker, 8);
        assert!(matches!(
            broker.reconcile_pending(&review, preserve()),
            Err(BrokerError::Io {
                operation: "injected writer interruption",
                ..
            })
        ));
        let entry = fs::read_dir(f.data.join("snapshots"))
            .unwrap()
            .map(Result::unwrap)
            .find(|e| e.file_name().to_string_lossy().ends_with("-proposed.bin"))
            .unwrap()
            .path();
        if damage == "bytes" {
            fs::write(entry, b"forged").unwrap();
        } else {
            fs::remove_file(entry).unwrap();
        }
        drop(broker);
        // 封存证据被篡改按状态损坏拒绝；被删除则按缺失的证据对象拒绝。重开都不能解锁。
        let result = FsVaultBroker::open_existing(&f.root, &f.data);
        let rejected = match damage {
            "bytes" => matches!(&result, Err(BrokerError::CorruptWriterState)),
            _ => matches!(
                &result,
                Err(BrokerError::NotFound(evidence)) if evidence.as_str().ends_with("-proposed.bin")
            ),
        };
        assert!(rejected, "{damage}: {:?}", result.err());
    }
}

#[test]
fn reconciliation_invalidates_old_capture_even_without_advancing_committed_watermark() {
    let f = Fixture::new();
    let mut broker = f.initialize();
    let first = broker
        .mutate(Mutation::create(path(), b"old".to_vec(), "first"))
        .unwrap();
    let old = broker.capture(ScanLimits::default()).unwrap().stamp;
    interrupt(&broker, 3);
    assert!(matches!(
        broker.mutate(Mutation::replace(
            path(),
            b"proposed".to_vec(),
            first.revision,
            "pending"
        )),
        Err(BrokerError::Io {
            operation: "injected writer interruption",
            ..
        })
    ));
    let review = broker.pending_mutation().unwrap().unwrap();
    broker.reconcile_pending(&review, preserve()).unwrap();
    assert!(matches!(
        broker.revalidate_capture(&old),
        Err(BrokerError::CaptureChanged)
    ));
    assert_eq!(
        broker
            .capture(ScanLimits::default())
            .unwrap()
            .stamp
            .committed_through(),
        1
    );
}
