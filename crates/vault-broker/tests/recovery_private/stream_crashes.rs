//! 保全后的中断只沿原终态恢复；未引用 v2 证据不能解锁旧 Installing。
use super::{stream_support::*, *};
use crate::MaintenanceDecision;

#[test]
fn direct_v2_evidence_and_terminal_interruptions_keep_unknown_and_upgrade_only_on_finish() {
    for point in [7, 8, 9] {
        let f = Fixture::new();
        let (mut broker, request, op, original) = old_pending(&f, LARGE);
        let review = broker.pending_mutation().unwrap().unwrap();
        interrupt(&broker, point);
        assert!(broker.reconcile_pending(&review, preserve()).is_err());
        // 此时 v2 包已经完整，但 State 仍是旧 Installing，水位从未变成成功。
        let state = crate::mutation::load(&broker).unwrap();
        assert_eq!(state.format_version, 2);
        assert!(state.operation.is_some());
        assert_eq!(state.committed_through, 1);
        drop(broker);
        let mut broker = f.open();
        if point == 7 {
            assert_eq!(
                broker.write_status().unwrap().phase,
                WriterPhase::Installing
            );
            assert!(broker.write_status().unwrap().read_blocked);
            assert!(broker.mutation_replay(&request).unwrap().is_none());
            assert!(matches!(
                broker.mutate(request.clone()),
                Err(BrokerError::MutationUnresolved)
            ));
            let next = broker.pending_mutation().unwrap().unwrap();
            broker.reconcile_pending(&next, preserve()).unwrap();
        }
        assert_ready(&broker);
        assert_eq!(crate::mutation::load(&broker).unwrap().format_version, 4);
        assert_eq!(envelope(&broker, &op), original);
        let Err(BrokerError::MutationIndeterminate { evidence_id }) =
            broker.mutation_replay(&request)
        else {
            panic!("原键必须保持未知终态");
        };
        assert_package(&f, &broker, &evidence_id, &op, LARGE, 2);
    }
}

#[test]
fn maintenance_reopen_keeps_exact_intent_review_and_never_calls_unreferenced_evidence_success() {
    for point in [7, 8, 9] {
        let f = Fixture::new();
        let (mut broker, request, op, original) = old_pending(&f, LARGE);
        let target = broker.maintenance_inspect(64).unwrap().targets.remove(0);
        let ticket = broker
            .prepare_maintenance(
                &target.review_id,
                MaintenanceDecision::PreserveInstalling,
                actor(),
            )
            .unwrap();
        broker
            .confirm_maintenance_intent(&ticket, 1, &"a".repeat(64))
            .unwrap();
        let intent = ticket.intent().clone();
        interrupt(&broker, point);
        assert!(broker.commit_maintenance(ticket).is_err());
        let state = crate::mutation::load(&broker).unwrap();
        assert_eq!(state.format_version, 3);
        assert!(state.operation.is_some());
        drop(broker);
        let mut broker = f.open();
        let pending = broker.maintenance_pending().unwrap().unwrap();
        assert_eq!(pending.intent, intent);
        assert!(pending.execution_started);
        if point == 7 {
            assert!(pending.outcome.is_none());
            assert_eq!(
                broker.write_status().unwrap().phase,
                WriterPhase::Installing
            );
            assert!(broker.mutation_replay(&request).unwrap().is_none());
            let ticket = broker
                .prepare_maintenance(&intent.target.review_id, intent.decision, actor())
                .unwrap();
            assert_eq!(ticket.intent(), &intent);
            broker.commit_maintenance(ticket).unwrap();
        }
        let pending = broker.maintenance_pending().unwrap().unwrap();
        assert_eq!(pending.intent, intent);
        let outcome = pending.outcome.unwrap();
        assert_eq!(outcome.target, target);
        assert_eq!(
            outcome.resolution,
            Some(ReconciliationOutcome::Indeterminate)
        );
        assert_package(
            &f,
            &broker,
            outcome.evidence_id.as_ref().unwrap(),
            &op,
            LARGE,
            2,
        );
        assert_eq!(crate::mutation::load(&broker).unwrap().format_version, 4);
        assert_eq!(envelope(&broker, &op), original);
        assert!(matches!(
            broker.mutation_replay(&request),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
        ack_maintenance(&mut broker);
        assert_ready(&broker);
    }
}
