use super::*;

#[test]
fn vec_restore_intent_blocks_new_keys_before_and_after_ack_but_original_can_commit() {
    for acknowledged in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        let (source, expected) = versions(&mut broker);
        ack(&mut broker);
        let (request, metadata) = restore_request(source, expected, "restore");
        let ticket = broker.prepare_recorded_restore(request, metadata).unwrap();
        if acknowledged {
            broker
                .confirm_recorded_restore_intent(&ticket, 1, &"a".repeat(64))
                .unwrap();
        }
        assert_blocked(&fixture, &mut broker);
        assert_old_replays(&fixture, &mut broker, source);
        assert!(
            broker
                .restore_replay(&path(), expected, source, "restore", &actor())
                .unwrap()
                .is_none()
        );
        if !acknowledged {
            broker
                .confirm_recorded_restore_intent(&ticket, 1, &"a".repeat(64))
                .unwrap();
        }
        assert_eq!(
            broker.commit_recorded_restore(ticket).unwrap().revision,
            source
        );
        assert_eq!(fixture.note(), b"A");
        assert_blocked(&fixture, &mut broker);
        ack(&mut broker);
        assert_resumed(&fixture, &mut broker);
    }
}

#[test]
fn committed_vec_and_stream_restore_gate_survives_reopen_without_blocking_exact_replay() {
    for streaming in [false, true] {
        for reopened in [false, true] {
            let fixture = Fixture::new();
            let mut broker = fixture.open();
            let (source, expected) = versions(&mut broker);
            ack(&mut broker);
            let receipt = install_restore(&mut broker, source, expected, streaming, false).unwrap();
            if reopened {
                drop(broker);
                broker = fixture.reopen();
            }
            assert_blocked(&fixture, &mut broker);
            assert_old_replays(&fixture, &mut broker, source);
            let before = tree(fixture._temp.path());
            assert_eq!(
                broker
                    .restore_replay(&path(), expected, source, "restore", &actor())
                    .unwrap(),
                Some(receipt.clone())
            );
            assert_eq!(
                replay_restore(&broker, source, expected, streaming).unwrap(),
                Some(receipt)
            );
            assert_eq!(tree(fixture._temp.path()), before);
            ack(&mut broker);
            assert_resumed(&fixture, &mut broker);
        }
    }
}

#[test]
fn reconciled_indeterminate_restore_keeps_new_write_gate_and_never_replays_success() {
    for streaming in [false, true] {
        let fixture = Fixture::new();
        let mut broker = fixture.open();
        let (source, expected) = versions(&mut broker);
        ack(&mut broker);
        assert!(install_restore(&mut broker, source, expected, streaming, true).is_err());
        assert_eq!(
            broker.write_status().unwrap().phase,
            WriterPhase::Installing
        );
        let review = broker.pending_mutation().unwrap().unwrap();
        let result = broker
            .reconcile_pending(
                &review,
                PendingDecision::PreserveCurrent {
                    original_content: b"A".to_vec(),
                },
            )
            .unwrap();
        assert_eq!(result.outcome, ReconciliationOutcome::Indeterminate);
        drop(broker);
        let mut broker = fixture.reopen();
        assert_blocked(&fixture, &mut broker);
        assert_old_replays(&fixture, &mut broker, source);
        let before = tree(fixture._temp.path());
        assert!(matches!(
            broker.restore_replay(&path(), expected, source, "restore", &actor()),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
        assert!(matches!(
            replay_restore(&broker, source, expected, streaming),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
        assert_eq!(tree(fixture._temp.path()), before);
        ack(&mut broker);
        assert_resumed(&fixture, &mut broker);
    }
}
