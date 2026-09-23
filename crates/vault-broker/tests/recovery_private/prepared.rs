use super::*;

#[test]
fn prepared_is_readable_but_keeps_other_writes_blocked() {
    let f = Fixture::new();
    let (mut broker, _) = f.interrupted(1);
    let status = broker.write_status().unwrap();
    assert_eq!(status.phase, WriterPhase::Prepared);
    assert!(status.unresolved);
    assert!(!status.read_blocked);
    assert_eq!(
        broker
            .capture(ScanLimits::default())
            .unwrap()
            .stamp
            .committed_through(),
        1
    );
    assert_eq!(broker.read_file(&path(), 32).unwrap().content, b"old");
    assert!(matches!(
        broker.mutate(Mutation::create(
            VaultPath::parse("other.md").unwrap(),
            vec![],
            "other"
        )),
        Err(BrokerError::MutationUnresolved)
    ));
    drop(broker);
    let reopened = f.open();
    assert_eq!(reopened.read_file(&path(), 32).unwrap().content, b"old");
    assert_eq!(
        reopened.write_status().unwrap().phase,
        WriterPhase::Prepared
    );
}

#[test]
fn cancel_is_permanent_without_claiming_conflict_or_a_commit() {
    let f = Fixture::new();
    let (mut broker, request) = f.interrupted(1);
    let review = broker.pending_mutation().unwrap().unwrap();
    assert_eq!(review.phase(), WriterPhase::Prepared);
    assert_eq!(review.previous_revision(), review.current_revision());
    let receipt = broker
        .reconcile_pending(&review, PendingDecision::CancelPrepared)
        .unwrap();
    assert_eq!(receipt.outcome, ReconciliationOutcome::Cancelled);
    assert_eq!(receipt.committed_through, 1);
    assert!(receipt.evidence_id.is_none());
    assert_ready(&broker);
    assert!(
        !f.root
            .join(".workbench/staging")
            .read_dir()
            .unwrap()
            .any(|_| true)
    );
    assert!(matches!(
        broker.mutate(request.clone()),
        Err(BrokerError::MutationCancelled)
    ));
    assert!(matches!(
        broker.mutation_replay(&request),
        Err(BrokerError::MutationCancelled)
    ));
    assert!(matches!(
        broker.reconcile_pending(&review, PendingDecision::CancelPrepared),
        Err(BrokerError::CaptureChanged)
    ));
    drop(broker);
    let mut reopened = f.open();
    assert_ready(&reopened);
    assert!(matches!(
        reopened.mutate(request),
        Err(BrokerError::MutationCancelled)
    ));
    assert_eq!(reopened.read(&path()).unwrap().content, b"old");
}

#[test]
fn cancelled_terminal_reopens_after_terminal_or_cleanup_interruptions() {
    for point in [8, 9] {
        let f = Fixture::new();
        let (mut broker, request) = f.interrupted(1);
        let review = broker.pending_mutation().unwrap().unwrap();
        interrupt(&broker, point);
        assert!(matches!(
            broker.reconcile_pending(&review, PendingDecision::CancelPrepared),
            Err(BrokerError::Io {
                operation: "injected writer interruption",
                ..
            })
        ));
        drop(broker);
        let mut reopened = f.open();
        assert_ready(&reopened);
        assert!(matches!(
            reopened.mutate(request),
            Err(BrokerError::MutationCancelled)
        ));
        assert_eq!(reopened.read(&path()).unwrap().content, b"old");
    }
}

#[test]
fn wrong_decision_and_changed_original_input_consume_the_review_without_unlocking() {
    for point in [1, 2] {
        let f = Fixture::new();
        let (mut broker, _) = f.interrupted(point);
        let review = broker.pending_mutation().unwrap().unwrap();
        let decision = if point == 1 {
            preserve()
        } else {
            PendingDecision::PreserveCurrent {
                original_content: b"wrong".to_vec(),
            }
        };
        // 对仅 Prepared 的操作选择保全属于决定不适用；原始内容不符则与原请求冲突。
        let result = broker.reconcile_pending(&review, decision);
        let rejected = if point == 1 {
            matches!(&result, Err(BrokerError::MutationUnresolved))
        } else {
            matches!(&result, Err(BrokerError::IdempotencyConflict))
        };
        assert!(rejected, "point {point}: {:?}", result.err());
        assert!(broker.write_status().unwrap().unresolved);
        assert!(matches!(
            broker.reconcile_pending(&review, PendingDecision::CancelPrepared),
            Err(BrokerError::CaptureChanged)
        ));
    }
}
