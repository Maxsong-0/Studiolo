//! 待办视图只认账本里的事实：有准入没结果才是待办，序号全账本唯一。
use super::*;
use serde_json::json;
use std::num::NonZeroU64;
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, IntegrityHash, MemoryAuditLedger, SanitizedPayload,
};
use studiolo_backup::worker::{
    job::backup_failure, restic::UnverifiedReason, state::FailureReason,
};
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
/// worker 写的结果事件（和 `backup_failure` 同一个事件键）；只放判断要看的字段。
fn outcome(ledger: &MemoryAuditLedger, key: &JobKey, body: serde_json::Value) -> AuditRecord {
    let mut body = body;
    body["job_sequence"] = json!(key.sequence.get());
    body["checkpoint_id"] = json!(key.checkpoint_id.as_str());
    ledger
        .append(AuditEvent::new(
            AuditEventId::parse(format!(
                "backup.snapshot:{}:{}",
                key.sequence,
                key.checkpoint_id.as_str()
            ))
            .unwrap(),
            AuditPhase::Outcome,
            BACKUP_AUDIT_ACTION,
            SanitizedPayload::try_from(body).unwrap(),
        ))
        .unwrap()
}
fn succeed(ledger: &MemoryAuditLedger, key: &JobKey, verified: bool) -> AuditRecord {
    outcome(
        ledger,
        key,
        json!({"result":"succeeded","objects_complete":true,"checksum_verified":verified}),
    )
}
fn at(record: &AuditRecord) -> i64 {
    record.recorded_at().as_millisecond()
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

#[test]
fn outcomes_record_the_latest_success_and_failure_in_ledger_order() {
    let ledger = MemoryAuditLedger::new();
    let first = intent(&ledger, &key(1), TARGET);
    admit(&ledger, &key(1), &first);
    let queue = view(&ledger);
    assert_eq!(
        queue.oldest_open(),
        Some(OpenJob {
            sequence: 1,
            since_ms: at(&first),
            admitted: true
        })
    );
    assert_eq!(queue.first_attempt_ms(), Some(at(&first)));
    assert_eq!(queue.last_success(), None);

    let done = succeed(&ledger, &key(1), true);
    let queue = view(&ledger);
    assert_eq!(
        queue.last_success(),
        Some(JobMark {
            sequence: 1,
            at_ms: at(&done)
        })
    );
    assert!(queue.oldest_open().is_none());
    assert!(!queue.latest_failed());

    intent(&ledger, &key(2), TARGET);
    let failed = fail(&ledger, &key(2));
    let queue = view(&ledger);
    assert!(queue.latest_failed());
    assert_eq!(
        queue.last_failure(),
        Some(FailureMark {
            sequence: 2,
            at_ms: at(&failed),
            failure: JobFailure::Failed(FailureReason::BarrierNotEstablished),
        })
    );
    assert_eq!(
        queue.last_success().map(|mark| mark.sequence),
        Some(1),
        "失败不抹掉之前的成功"
    );
    assert_eq!(
        queue.first_attempt_ms(),
        Some(at(&first)),
        "第一次尝试的时刻不随后来的尝试变"
    );

    intent(&ledger, &key(3), TARGET);
    succeed(&ledger, &key(3), true);
    let queue = view(&ledger);
    assert!(!queue.latest_failed(), "之后又成功了");
    assert_eq!(
        queue.last_failure().map(|mark| mark.sequence),
        Some(2),
        "之前的失败留着给人看"
    );
}

#[test]
fn a_success_without_verified_objects_or_an_unreadable_result_is_not_a_success() {
    let ledger = MemoryAuditLedger::new();
    intent(&ledger, &key(1), TARGET);
    succeed(&ledger, &key(1), false);
    let queue = view(&ledger);
    assert_eq!(queue.last_success(), None, "没写明校验通过就不算成功");
    assert_eq!(
        queue.last_failure().map(|mark| mark.failure),
        Some(JobFailure::Unrecognized)
    );
    assert!(queue.latest_failed());
    assert!(!queue.outstanding(), "照样收口，不会一直挂着");

    intent(&ledger, &key(2), TARGET);
    outcome(
        &ledger,
        &key(2),
        json!({"result":"unverified","reason":"target_unreachable"}),
    );
    intent(&ledger, &key(3), TARGET);
    outcome(
        &ledger,
        &key(3),
        json!({"result":"failed","reason":"no_such_reason"}),
    );
    let queue = view(&ledger);
    assert_eq!(
        queue
            .last_failure()
            .map(|mark| (mark.sequence, mark.failure)),
        Some((3, JobFailure::Unrecognized)),
        "认不出的原因不猜"
    );
    let ledger = MemoryAuditLedger::new();
    intent(&ledger, &key(2), TARGET);
    outcome(
        &ledger,
        &key(2),
        json!({"result":"unverified","reason":"target_unreachable"}),
    );
    assert_eq!(
        view(&ledger).last_failure().map(|mark| mark.failure),
        Some(JobFailure::Unverified(UnverifiedReason::TargetUnreachable))
    );
}

#[test]
fn outcomes_that_close_nothing_here_are_neither_success_nor_failure() {
    let ledger = MemoryAuditLedger::new();
    // 别的目标类别的作业收口了：不算这里的成功，它的尝试也不算这里的第一次尝试。
    intent(&ledger, &key(1), BackupTargetKind::LocalTemporaryRepository);
    succeed(&ledger, &key(1), true);
    // 这里的作业，收到的却是另一个 checkpoint 的结果：不收口，也不算失败。
    let mine = intent(&ledger, &key(2), TARGET);
    fail(
        &ledger,
        &JobKey {
            sequence: NonZeroU64::new(2).unwrap(),
            checkpoint_id: CheckpointId::parse("someone-else").unwrap(),
        },
    );
    let queue = view(&ledger);
    assert_eq!(queue.last_success(), None);
    assert_eq!(queue.last_failure(), None);
    assert!(!queue.latest_failed());
    assert_eq!(
        queue.oldest_open(),
        Some(OpenJob {
            sequence: 2,
            since_ms: at(&mine),
            admitted: false
        })
    );
    assert_eq!(queue.first_attempt_ms(), Some(at(&mine)));
}
