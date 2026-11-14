use super::*;
use std::num::NonZeroU64;
use studiolo_core::{AttemptId, TaskId, checkpoint::CheckpointId};
#[test]
fn only_one_original_job_phase_can_be_queried() {
    let key = JobKey {
        sequence: NonZeroU64::new(1).unwrap(),
        checkpoint_id: CheckpointId::parse("fixed-audit-job").unwrap(),
    };
    let target = BackupTargetKind::DedicatedAppendOnlyTarget;
    let id = backup_intent(&key, target).unwrap().event_id().clone();
    let exact = AuditFilter::default()
        .with_event_id(id)
        .with_phase(AuditPhase::Intent)
        .with_limit(1);
    assert_eq!(
        query_phase(&exact, &key, target).unwrap(),
        AuditPhase::Intent
    );
    assert_eq!(
        query_phase(&exact.clone().with_phase(AuditPhase::Outcome), &key, target).unwrap(),
        AuditPhase::Outcome
    );
    for filter in [
        AuditFilter::default(),
        exact.clone().with_limit(0),
        exact.clone().with_limit(2),
        exact
            .clone()
            .with_event_id(studiolo_audit_ledger::AuditEventId::parse("another").unwrap()),
        exact.clone().with_action("policy.activate"),
        exact.clone().after_offset(0),
        exact.clone().with_task(TaskId::new()),
        exact.clone().with_attempt(AttemptId::new()),
    ] {
        assert!(query_phase(&filter, &key, target).is_err());
    }
}
