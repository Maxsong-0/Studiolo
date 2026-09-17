//! 旧 Installing 的对象预算兼容；不改变原请求或维护审阅。
use super::{stream_support::*, *};
use crate::MaintenanceDecision;

#[test]
fn old_small_installing_uses_actual_current_budget_and_preserves_original_envelope() {
    for size in [crate::writer::disk::CONTENT_LIMIT, LARGE] {
        let f = Fixture::new();
        let (mut broker, request, op, original) = old_pending(&f, size);
        let review = broker.pending_mutation().unwrap().unwrap();
        let receipt = broker.reconcile_pending(&review, preserve()).unwrap();
        let large = size > crate::writer::disk::CONTENT_LIMIT;
        let id = receipt.evidence_id.unwrap();
        assert_eq!(receipt.outcome, ReconciliationOutcome::Indeterminate);
        assert_eq!(receipt.committed_through, 1);
        assert_package(&f, &broker, &id, &op, size, if large { 2 } else { 1 });
        assert_eq!(
            crate::mutation::load(&broker).unwrap().format_version,
            if large { 4 } else { 2 }
        );
        assert_eq!(envelope(&broker, &op), original);
        assert!(matches!(
            broker.mutation_replay(&request),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
        drop(broker);
        let mut reopened = f.open();
        assert_ready(&reopened);
        assert!(matches!(
            reopened.mutate(request),
            Err(BrokerError::MutationIndeterminate { .. })
        ));
        assert_eq!(envelope(&reopened, &op), original);
    }
}

#[test]
fn maintenance_keeps_original_review_through_state3_then_promotes_v2_terminal_to_state4() {
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
    assert_eq!(ticket.intent().target, target);
    assert_eq!(crate::mutation::load(&broker).unwrap().format_version, 3);
    broker
        .confirm_maintenance_intent(&ticket, 1, &"a".repeat(64))
        .unwrap();
    let receipt = broker.commit_maintenance(ticket).unwrap();
    assert_eq!(receipt.event.target, target);
    assert_eq!(
        receipt.event.resolution,
        Some(ReconciliationOutcome::Indeterminate)
    );
    assert_eq!(receipt.committed_through, 1);
    assert_package(
        &f,
        &broker,
        receipt.event.evidence_id.as_ref().unwrap(),
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
    drop(broker);
    let reopened = f.open();
    assert_ready(&reopened);
    assert_eq!(crate::mutation::load(&reopened).unwrap().format_version, 4);
}

#[test]
fn state4_small_evidence_does_not_downgrade_to_manifest1() {
    let f = Fixture::new();
    let (mut broker, _, op, _) = old_pending(&f, 64);
    let mut state = crate::mutation::load(&broker).unwrap();
    state.format_version = 4;
    crate::mutation::save(&broker, &state).unwrap();
    let review = broker.pending_mutation().unwrap().unwrap();
    let receipt = broker.reconcile_pending(&review, preserve()).unwrap();
    assert_package(
        &f,
        &broker,
        receipt.evidence_id.as_ref().unwrap(),
        &op,
        64,
        2,
    );
    assert_eq!(crate::mutation::load(&broker).unwrap().format_version, 4);
}

#[test]
fn large_object_cannot_be_relabeled_as_v1_and_unknown_manifest_version_stays_closed() {
    let f = Fixture::new();
    let (mut broker, _, op, _) = old_pending(&f, LARGE);
    let review = broker.pending_mutation().unwrap().unwrap();
    let receipt = broker.reconcile_pending(&review, preserve()).unwrap();
    let original_id = receipt.evidence_id.unwrap();
    let snapshots = f.data.join("snapshots");
    let original = fs::read(snapshots.join(format!("recovery-{original_id}.json"))).unwrap();
    for version in [1, 99] {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        value["version"] = version.into();
        let encoded = serde_json::to_vec(&value).unwrap();
        let id = crate::sha256(&encoded);
        fs::write(snapshots.join(format!("recovery-{id}.json")), encoded).unwrap();
        for role in ["proposed", "current", "previous"] {
            fs::copy(
                snapshots.join(format!("recovery-{original_id}-{role}.bin")),
                snapshots.join(format!("recovery-{id}-{role}.bin")),
            )
            .unwrap();
        }
        assert!(matches!(
            super::super::evidence::verify(&broker, &id, &op, receipt.current_revision),
            Err(BrokerError::CorruptWriterState)
        ));
    }
}
