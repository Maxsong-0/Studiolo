//! 待办视图只认账本里的事实：有准入没结果才是待办，序号全账本唯一。
use super::*;
use serde_json::json;
use std::num::NonZeroU64;
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, IntegrityHash, MemoryAuditLedger, SanitizedPayload,
};
use studiolo_backup::worker::{job::backup_failure, state::FailureReason};
use studiolo_core::checkpoint::CheckpointId;

const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;

fn key(sequence: u64) -> JobKey {
    JobKey {
        sequence: NonZeroU64::new(sequence).unwrap(),
        checkpoint_id: CheckpointId::parse(format!("queue-{sequence}")).unwrap(),
    }
}
fn intent(ledger: &MemoryAuditLedger, key: &JobKey, target: BackupTargetKind) -> AuditRecord {
    ledger.append(backup_intent(key, target).unwrap()).unwrap()
}
/// 形状完整的准入事件：以原意图那条记录为审计截止点。
fn admit(ledger: &MemoryAuditLedger, key: &JobKey, cutoff: &AuditRecord) -> AuditRecord {
    let hash = IntegrityHash::sha256(key.checkpoint_id.as_str().as_bytes()).to_hex();
    let body = json!({
        "schema":1,"key":key,"target":TARGET,
        "source":{"checkpoint_id":key.checkpoint_id,"seal_sha256":"ab".repeat(32)},
        "content_cutoff":0,"expires_at":i64::MAX,
        "audit_offset":cutoff.offset(),"audit_hash":cutoff.record_hash().to_hex(),
    });
    ledger
        .append(AuditEvent::new(
            AuditEventId::parse(format!("backup-admission-{hash}")).unwrap(),
            AuditPhase::Intent,
            BACKUP_AUDIT_ACTION,
            SanitizedPayload::try_from(body).unwrap(),
        ))
        .unwrap()
}
fn fail(ledger: &MemoryAuditLedger, key: &JobKey) -> AuditRecord {
    ledger
        .append(backup_failure(key, FailureReason::BarrierNotEstablished).unwrap())
        .unwrap()
}
fn view(ledger: &MemoryAuditLedger) -> Queue {
    let mut queue = Queue::default();
    for record in ledger
        .query(&studiolo_audit_ledger::AuditFilter::default())
        .unwrap()
    {
        queue.absorb(TARGET, &record);
    }
    queue
}

#[test]
fn an_empty_ledger_has_nothing_pending_and_starts_at_one() {
    let queue = Queue::default();
    assert_eq!(queue.next_sequence(), Some(1));
    assert!(queue.next_pending().is_none());
    assert!(!queue.outstanding());
    assert!(queue.orphans().is_empty());
    assert_eq!(queue.last_attempt_ms(), None);
}

#[test]
fn only_an_admitted_key_without_an_outcome_is_pending_and_the_oldest_comes_first() {
    let ledger = MemoryAuditLedger::new();
    let first = intent(&ledger, &key(1), TARGET);
    // 只有原意图：算没做完，但不是待办，也不能交给 worker。
    let queue = view(&ledger);
    assert!(queue.outstanding());
    assert!(queue.next_pending().is_none());
    assert_eq!(queue.orphans(), vec![key(1)]);
    assert_eq!(queue.next_sequence(), Some(2));
    assert_eq!(queue.last_admission_ms(), None);
    assert!(queue.last_attempt_ms().is_some());

    let admitted = admit(&ledger, &key(1), &first);
    let second = intent(&ledger, &key(2), TARGET);
    admit(&ledger, &key(2), &second);
    let queue = view(&ledger);
    assert_eq!(queue.next_pending(), Some(&admitted), "先做最早的那个");
    assert!(queue.orphans().is_empty());
    assert!(queue.last_admission_ms().is_some());
    assert_eq!(queue.scanned(), 4);

    fail(&ledger, &key(1));
    let queue = view(&ledger);
    let pending = queue.next_pending().expect("第二个还在");
    assert_eq!(AdmissionFact::from_receipt(pending).unwrap().key(), &key(2));
    fail(&ledger, &key(2));
    let queue = view(&ledger);
    assert!(!queue.outstanding());
    assert!(queue.next_pending().is_none());
    assert_eq!(queue.next_sequence(), Some(3), "做完的序号不复用");
}

#[test]
fn other_targets_and_unrelated_events_only_move_the_sequence_and_position() {
    let ledger = MemoryAuditLedger::new();
    // 别的目标类别的作业：占序号，但不是这里的待办，也不算一次尝试。
    intent(&ledger, &key(9), BackupTargetKind::LocalTemporaryRepository);
    // 别的动作：只推进位置。
    ledger
        .append(AuditEvent::new(
            AuditEventId::parse("unrelated-1").unwrap(),
            AuditPhase::Intent,
            "policy.activate",
            SanitizedPayload::try_from(json!({"job_sequence":40,"checkpoint_id":"queue-40"}))
                .unwrap(),
        ))
        .unwrap();
    let queue = view(&ledger);
    assert_eq!(queue.next_sequence(), Some(10));
    assert!(!queue.outstanding());
    assert_eq!(queue.last_attempt_ms(), None);
    assert_eq!(queue.scanned(), 2);
}

#[test]
fn an_outcome_for_another_checkpoint_does_not_close_the_job() {
    let ledger = MemoryAuditLedger::new();
    let first = intent(&ledger, &key(1), TARGET);
    admit(&ledger, &key(1), &first);
    // 同序号、另一个 checkpoint 的结果：不是这条作业的。
    let stranger = JobKey {
        sequence: NonZeroU64::new(1).unwrap(),
        checkpoint_id: CheckpointId::parse("someone-else").unwrap(),
    };
    fail(&ledger, &stranger);
    assert!(view(&ledger).next_pending().is_some());
}
