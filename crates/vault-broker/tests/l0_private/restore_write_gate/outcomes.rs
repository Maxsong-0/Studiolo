use super::*;

#[test]
fn cancelled_and_rejected_restore_outcomes_do_not_require_ack_for_new_ordinary_writes() {
    for streaming in [false, true] {
        for cancelled in [false, true] {
            let fixture = Fixture::new();
            let mut broker = fixture.open();
            let (source, current) = versions(&mut broker);
            ack(&mut broker);
            // 拒绝分支故意使用 A 的旧条件，当前仍是 B；不需要外部改盘或竞争。
            let expected = if cancelled { current } else { source };
            if streaming {
                let result = broker.prepare_stream_restore(
                    path(),
                    expected,
                    source,
                    "restore".into(),
                    actor(),
                    StreamLimits::new(16).unwrap(),
                );
                if cancelled {
                    let ticket = result.unwrap();
                    broker
                        .confirm_stream_restore_intent(&ticket, 1, &"a".repeat(64))
                        .unwrap();
                    broker.cancel_stream_mutation(ticket).unwrap();
                } else {
                    assert!(matches!(result, Err(BrokerError::RevisionConflict { .. })));
                }
            } else {
                let (request, metadata) = restore_request(source, expected, "restore");
                let ticket = broker.prepare_recorded_restore(request, metadata).unwrap();
                if cancelled {
                    broker.cancel_recorded_restore(ticket).unwrap();
                } else {
                    broker
                        .confirm_recorded_restore_intent(&ticket, 1, &"a".repeat(64))
                        .unwrap();
                    assert!(matches!(
                        broker.commit_recorded_restore(ticket),
                        Err(BrokerError::RevisionConflict { .. })
                    ));
                }
            }
            let status = broker.content_audit_status().unwrap();
            assert!(status.pending > 0);
            assert!(!status.restore_unconfirmed);
            assert_eq!(fixture.note(), b"B");
            let before = tree(fixture._temp.path());
            let result = broker.restore_replay(&path(), expected, source, "restore", &actor());
            if cancelled {
                assert!(matches!(result, Err(BrokerError::MutationCancelled)));
            } else {
                assert!(matches!(result, Err(BrokerError::RevisionConflict { .. })));
            }
            assert_eq!(tree(fixture._temp.path()), before);
            assert_resumed(&fixture, &mut broker);
        }
    }
}

#[test]
fn pending_ordinary_replace_audit_keeps_its_existing_fail_open_rule() {
    let fixture = Fixture::new();
    let mut broker = fixture.open();
    versions(&mut broker);
    assert!(broker.content_audit_status().unwrap().pending > 0);
    assert_resumed(&fixture, &mut broker);
}
