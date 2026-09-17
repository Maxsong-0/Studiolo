use super::*;
#[test]
fn idle_and_enabled_initialization_interruptions_never_start_a_decision() {
    for point in [30, 31] {
        let fixture = Fixture::new();
        let (mut broker, request) = fixture.pending(1);
        let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
        fault(&broker, point);
        assert!(
            broker
                .prepare_maintenance(
                    &target.review_id,
                    MaintenanceDecision::CancelPrepared,
                    actor()
                )
                .is_err()
        );
        drop(broker);
        let mut broker = fixture.reopen();
        assert!(broker.maintenance_inspect(64).unwrap().pending.is_none());
        assert!(broker.mutation_replay(&request).unwrap().is_none());
        assert_eq!(fixture.body(), b"old");
        let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
        intent(&mut broker, &ticket);
        broker.commit_maintenance(ticket).unwrap();
        flush(&mut broker);
        assert!(!broker.write_status().unwrap().unresolved);
    }
}
#[test]
fn active_intent_reopen_and_flush_dont_execute_and_allow_explicit_slot_cancel() {
    let fixture = Fixture::new();
    let (mut broker, request) = fixture.pending(1);
    let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
    fault(&broker, 32);
    assert!(
        broker
            .prepare_maintenance(
                &target.review_id,
                MaintenanceDecision::CancelPrepared,
                actor()
            )
            .is_err()
    );
    drop(broker);
    let mut broker = fixture.reopen();
    flush(&mut broker);
    let pending = broker.maintenance_inspect(64).unwrap().pending.unwrap();
    assert!(pending.intent_confirmed);
    assert!(!pending.execution_started);
    assert!(pending.outcome.is_none());
    assert!(broker.mutation_replay(&request).unwrap().is_none());
    broker
        .cancel_maintenance(&pending.maintenance_id, actor())
        .unwrap();
    flush(&mut broker);
    assert_eq!(broker.write_status().unwrap().phase, WriterPhase::Prepared);
}
#[test]
fn executing_without_terminal_stays_unknown_and_requires_explicit_retry() {
    let fixture = Fixture::new();
    let (mut broker, request) = fixture.pending(1);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    intent(&mut broker, &ticket);
    let review_id = ticket.intent().target.review_id.clone();
    let id = ticket.intent().event_id.clone();
    fault(&broker, 33);
    assert!(broker.commit_maintenance(ticket).is_err());
    drop(broker);
    let mut broker = fixture.reopen();
    flush(&mut broker);
    assert!(broker.pending_maintenance_audit(64).unwrap().is_empty());
    assert!(
        broker
            .maintenance_inspect(64)
            .unwrap()
            .pending
            .unwrap()
            .execution_started
    );
    assert!(broker.mutation_replay(&request).unwrap().is_none());
    assert!(matches!(
        broker.cancel_maintenance(&id, actor()),
        Err(BrokerError::MutationUnresolved)
    ));
    let ticket = broker
        .prepare_maintenance(&review_id, MaintenanceDecision::CancelPrepared, actor())
        .unwrap();
    broker.commit_maintenance(ticket).unwrap();
    flush(&mut broker);
    assert!(!broker.write_status().unwrap().unresolved);
}
#[test]
fn terminal_and_outcome_crashes_only_backfill_the_fixed_executed_result() {
    for point in [8, 34, 35] {
        let fixture = Fixture::new();
        let (mut broker, request) = fixture.pending(3);
        let ticket = prepare(&mut broker, MaintenanceDecision::PreserveInstalling);
        intent(&mut broker, &ticket);
        let original = ticket.intent().clone();
        fault(&broker, point);
        assert!(broker.commit_maintenance(ticket).is_err());
        drop(broker);
        let mut broker = fixture.reopen();
        let pending = broker.maintenance_inspect(64).unwrap().pending.unwrap();
        assert_eq!(pending.intent, original);
        assert_eq!(
            pending.outcome.unwrap().resolution,
            Some(crate::ReconciliationOutcome::Indeterminate)
        );
        assert!(broker.write_status().unwrap().unresolved);
        assert_eq!(fixture.body(), b"proposed");
        assert!(matches!(
            broker.mutation_replay(&request),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
        flush(&mut broker);
        assert!(!broker.write_status().unwrap().unresolved);
    }
}
#[test]
fn outcome_ack_interruption_is_durably_complete_after_reopen() {
    let fixture = Fixture::new();
    let (mut broker, _) = fixture.pending(1);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    intent(&mut broker, &ticket);
    broker.commit_maintenance(ticket).unwrap();
    let candidate = broker.pending_maintenance_audit(64).unwrap().remove(0);
    fault(&broker, 36);
    assert!(
        broker
            .confirm_maintenance_audit(candidate, 2, &"a".repeat(64))
            .is_err()
    );
    drop(broker);
    let broker = fixture.reopen();
    assert!(!broker.write_status().unwrap().unresolved);
    assert!(broker.pending_maintenance_audit(64).unwrap().is_empty());
}
