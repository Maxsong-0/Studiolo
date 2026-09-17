use super::*;
fn human() -> crate::MutationActor {
    crate::MutationActor::Human {
        issuer: "https://issuer.test".into(),
        subject: "original-human".into(),
    }
}
fn metadata(kind: crate::MutationKind) -> crate::MutationMetadata {
    crate::MutationMetadata::new(human(), kind).unwrap()
}
fn restore(
    fixture: &Fixture,
) -> (
    FsVaultBroker,
    crate::RecordedRestore,
    crate::Revision,
    crate::Revision,
) {
    let mut broker = fixture.open();
    let source = broker
        .mutate_recorded(
            Mutation::create(path(), b"A".to_vec(), "first"),
            metadata(crate::MutationKind::Replace),
        )
        .unwrap()
        .revision;
    let expected = broker
        .mutate_recorded(
            Mutation::replace(path(), b"B".to_vec(), source, "second"),
            metadata(crate::MutationKind::Replace),
        )
        .unwrap()
        .revision;
    flush_content(&mut broker);
    let ticket = broker
        .prepare_recorded_restore(
            Mutation::replace(path(), b"A".to_vec(), expected, "restore"),
            metadata(crate::MutationKind::Restore {
                source_revision: source,
            }),
        )
        .unwrap();
    (broker, ticket, source, expected)
}
#[test]
fn lost_key_restore_cancel_keeps_original_actor_and_has_two_independent_outboxes() {
    let fixture = Fixture::new();
    let (mut broker, original, source, expected) = restore(&fixture);
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    assert_eq!(target.kind, MaintenanceTargetKind::RestoreIntent);
    assert_eq!(target.current, MaintenanceCurrent::NotObserved);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelRestoreIntent);
    assert!(matches!(
        broker.cancel_recorded_restore(original),
        Err(BrokerError::MutationUnresolved)
    ));
    intent(&mut broker, &ticket);
    let receipt = broker.commit_maintenance(ticket).unwrap();
    assert_eq!(
        receipt.event.resolution,
        Some(crate::ReconciliationOutcome::Cancelled)
    );
    assert_eq!(fixture.body(), b"B");
    assert!(!broker.content_audit_status().unwrap().restore_unconfirmed);
    for phase in [ContentAuditPhase::Intent, ContentAuditPhase::Outcome] {
        let events = broker.pending_content_audit(64).unwrap();
        assert_eq!(events.len(), 1);
        for candidate in events {
            assert_eq!(candidate.event().actor, human());
            assert_eq!(candidate.event().phase, phase);
            let offset = if phase == ContentAuditPhase::Intent {
                1
            } else {
                2
            };
            broker
                .confirm_content_audit(candidate, offset, &"a".repeat(64))
                .unwrap();
        }
    }
    assert!(broker.write_status().unwrap().unresolved);
    flush(&mut broker);
    assert!(!broker.write_status().unwrap().unresolved);
    assert!(matches!(
        broker.restore_replay(&path(), expected, source, "restore", &human()),
        Err(BrokerError::MutationCancelled)
    ));
}
#[test]
fn cancelling_maintenance_intent_does_not_cancel_original_restore_or_open_new_write() {
    let fixture = Fixture::new();
    let (mut broker, original, _, _) = restore(&fixture);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelRestoreIntent);
    intent(&mut broker, &ticket);
    let receipt = broker
        .cancel_maintenance(&ticket.intent().event_id, actor())
        .unwrap();
    assert_eq!(receipt.event.outcome, Some(MaintenanceOutcome::Cancelled));
    flush(&mut broker);
    assert!(broker.content_audit_status().unwrap().restore_unconfirmed);
    assert_eq!(broker.pending_recorded_restores(64).unwrap().len(), 1);
    // Broker 的内容门禁继续由受信 owner 检查；维护不解除原恢复审计未决。
    broker.cancel_recorded_restore(original).unwrap();
    assert!(!broker.content_audit_status().unwrap().restore_unconfirmed);
    assert_eq!(fixture.body(), b"B");
}
