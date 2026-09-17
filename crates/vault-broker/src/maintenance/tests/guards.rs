use super::*;
#[test]
fn active_slot_guards_unrecorded_write_preflight_reconciliation_and_pruning() {
    let fixture = Fixture::new();
    let (mut broker, request) = fixture.pending(1);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    let review = broker.pending_mutation().unwrap().unwrap();
    assert!(matches!(
        broker.mutate(request.clone()),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        broker.check_precondition(&request),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        broker.reconcile_pending(&review, crate::PendingDecision::CancelPrepared),
        Err(BrokerError::MutationUnresolved)
    ));
    assert!(matches!(
        broker.prune_stages(),
        Err(BrokerError::MutationUnresolved)
    ));
    assert_eq!(broker.read(&path()).unwrap().content, b"old");
    assert!(!broker.write_status().unwrap().read_blocked);
    assert!(
        broker
            .cancel_maintenance(&ticket.intent().event_id, actor())
            .is_err()
    );
}
#[test]
fn wrong_actor_action_and_cross_instance_ticket_never_authorize_maintenance() {
    let fixture = Fixture::new();
    let (mut broker, _) = fixture.pending(1);
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    let MaintenanceActor::LocalOs { uid, gid } = actor();
    let other = MaintenanceActor::LocalOs {
        uid: uid.wrapping_add(1),
        gid,
    };
    assert!(matches!(
        broker.prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::CancelPrepared,
            other
        ),
        Err(BrokerError::InvalidArtifact(_))
    ));
    assert!(matches!(
        broker.prepare_maintenance(
            &target.review_id,
            MaintenanceDecision::PreserveInstalling,
            actor()
        ),
        Err(BrokerError::InvalidArtifact(_))
    ));
    assert!(broker.maintenance_inspect(0).is_err());
    assert!(broker.maintenance_inspect(65).is_err());
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    let candidate = broker.pending_maintenance_audit(64).unwrap().remove(0);
    drop(broker);
    let mut broker = fixture.reopen();
    assert!(matches!(
        broker.confirm_maintenance_intent(&ticket, 1, &"a".repeat(64)),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(matches!(
        broker.confirm_maintenance_audit(candidate, 1, &"a".repeat(64)),
        Err(BrokerError::CaptureChanged)
    ));
}
#[test]
fn consumed_slot_generation_rejects_old_tickets_and_audit_candidates() {
    let fixture = Fixture::new();
    let (mut broker, _) = fixture.pending(1);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    let old_ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    let old_candidate = broker.pending_maintenance_audit(64).unwrap().remove(0);
    intent(&mut broker, &ticket);
    assert!(matches!(
        broker.confirm_maintenance_intent(&ticket, 2, &"b".repeat(64)),
        Err(BrokerError::IdempotencyConflict)
    ));
    broker
        .cancel_maintenance(&ticket.intent().event_id, actor())
        .unwrap();
    flush(&mut broker);
    let new = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    assert_ne!(new.intent().event_id, old_ticket.intent().event_id);
    assert!(matches!(
        broker.commit_maintenance(old_ticket),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(matches!(
        broker.confirm_maintenance_audit(old_candidate, 1, &"a".repeat(64)),
        Err(BrokerError::CaptureChanged)
    ));
    assert!(broker.maintenance_inspect(64).unwrap().pending.is_some());
}
