//! 队列命令没有作业键：答复种类必须属于那条命令，`pending` 只能带同目标的原准入回执。
use serde_json::json;
use std::num::NonZeroU64;
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditRecord, IntegrityHash,
    MemoryAuditLedger, SanitizedPayload,
};
use studiolo_backup::worker::{
    audit::{BackupAuditRequest, BackupQueueRequest, QueueAnswer, QueueCommand},
    job::{backup_event_key, backup_failure, backup_intent},
    recovery_point::BackupTargetKind,
    state::{FailureReason, JobKey},
};
use studiolo_core::checkpoint::CheckpointId;
const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;
fn key() -> JobKey {
    JobKey {
        sequence: NonZeroU64::new(7).unwrap(),
        checkpoint_id: CheckpointId::parse("queue-wire").unwrap(),
    }
}
/// 一条形状完整的准入回执：先落原意图，再落以它为截止点的准入事件。
fn admission(ledger: &MemoryAuditLedger, target: BackupTargetKind) -> AuditRecord {
    let intent = ledger
        .append(backup_intent(&key(), target).unwrap())
        .unwrap();
    let hash = IntegrityHash::sha256(key().checkpoint_id.as_str().as_bytes()).to_hex();
    let body = json!({
        "schema":1,"key":key(),"target":target,
        "source":{"checkpoint_id":key().checkpoint_id,"seal_sha256":"ab".repeat(32)},
        "content_cutoff":0,"expires_at":i64::MAX,
        "audit_offset":intent.offset(),"audit_hash":intent.record_hash().to_hex(),
    });
    ledger
        .append(AuditEvent::new(
            AuditEventId::parse(format!("backup-admission-{hash}")).unwrap(),
            AuditPhase::Intent,
            "backup.snapshot",
            SanitizedPayload::try_from(body).unwrap(),
        ))
        .unwrap()
}
#[test]
fn each_command_roundtrips_and_only_accepts_its_own_answers() {
    let ledger = MemoryAuditLedger::new();
    let pending = QueueAnswer::Pending(Box::new(admission(&ledger, TARGET)));
    let next = BackupQueueRequest::new(TARGET, QueueCommand::NextPending);
    let schedule = BackupQueueRequest::new(TARGET, QueueCommand::Schedule);
    for (request, allowed, refused) in [
        (
            &next,
            vec![pending.clone(), QueueAnswer::Idle],
            vec![
                QueueAnswer::Started,
                QueueAnswer::Outstanding,
                QueueAnswer::NotDue,
            ],
        ),
        (
            &schedule,
            vec![
                QueueAnswer::Started,
                QueueAnswer::Outstanding,
                QueueAnswer::NotDue,
            ],
            vec![pending.clone(), QueueAnswer::Idle],
        ),
    ] {
        let bytes = request.encode().unwrap();
        let decoded = BackupQueueRequest::decode(&bytes).unwrap();
        assert_eq!(decoded.command(), request.command());
        assert_eq!(decoded.target(), TARGET);
        for answer in allowed {
            let reply = decoded.encode_reply(&answer).unwrap();
            assert_eq!(request.decode_reply(&reply).unwrap(), answer);
        }
        for answer in refused {
            assert!(decoded.encode_reply(&answer).is_err(), "{answer:?}");
        }
    }
    // 答复绑定请求：拿「安排一次」的答复去答「下一个待办」，摘要对不上。
    let reply = schedule.encode_reply(&QueueAnswer::Started).unwrap();
    assert!(next.decode_reply(&reply).is_err());
}
#[test]
fn pending_must_carry_a_real_admission_for_the_same_target() {
    let ledger = MemoryAuditLedger::new();
    let other = admission(&ledger, BackupTargetKind::LocalTemporaryRepository);
    let next = BackupQueueRequest::new(TARGET, QueueCommand::NextPending);
    assert!(
        next.encode_reply(&QueueAnswer::Pending(Box::new(other)))
            .is_err(),
        "别的目标的准入不能当成本目标的待办"
    );
    // 普通意图不是准入回执。
    let plain = MemoryAuditLedger::new()
        .append(backup_intent(&key(), TARGET).unwrap())
        .unwrap();
    assert!(
        next.encode_reply(&QueueAnswer::Pending(Box::new(plain)))
            .is_err()
    );
    // 线上把 idle 改成带回执、或把 pending 的回执拿掉，都拒绝。
    let idle: serde_json::Value =
        serde_json::from_slice(&next.encode_reply(&QueueAnswer::Idle).unwrap()).unwrap();
    let mut forged = idle.clone();
    forged["answer"] = json!("pending");
    assert!(
        next.decode_reply(&serde_json::to_vec(&forged).unwrap())
            .is_err()
    );
    let mut extra = idle;
    extra["note"] = json!("x");
    assert!(
        next.decode_reply(&serde_json::to_vec(&extra).unwrap())
            .is_err()
    );
}
#[test]
fn queue_and_keyed_messages_cannot_be_decoded_as_each_other() {
    let queue = BackupQueueRequest::new(TARGET, QueueCommand::Schedule)
        .encode()
        .unwrap();
    assert!(BackupAuditRequest::decode(&queue).is_err());
    let keyed = BackupAuditRequest::admission(key(), TARGET)
        .encode()
        .unwrap();
    assert!(BackupQueueRequest::decode(&keyed).is_err());
    let mut spaced = queue.clone();
    spaced.push(b' ');
    assert!(BackupQueueRequest::decode(&spaced).is_err(), "非规范编码");
    assert!(BackupQueueRequest::decode(b"").is_err());
}
#[test]
fn the_original_key_is_read_back_only_from_job_events() {
    let ledger = MemoryAuditLedger::new();
    let intent = backup_intent(&key(), TARGET).unwrap();
    assert_eq!(backup_event_key(&intent), Some(key()));
    let failed = backup_failure(&key(), FailureReason::BarrierNotEstablished).unwrap();
    assert_eq!(failed.phase(), AuditPhase::Outcome);
    assert_eq!(backup_event_key(&failed), Some(key()));
    // 固定失败结果过得了带键协议的事件校验：和 worker 自己写的是同一个形状。
    assert!(BackupAuditRequest::append(key(), TARGET, failed).is_ok());
    // 准入事件用的是另一种事件键，这里读不出来。
    let admitted = admission(&ledger, TARGET);
    assert_eq!(backup_event_key(admitted.event()), None);
    // 事件键和载荷对不上的不算。
    let mut value = serde_json::to_value(&intent).unwrap();
    value["payload"]["job_sequence"] = json!(8);
    let changed: AuditEvent = serde_json::from_value(value).unwrap();
    assert_eq!(backup_event_key(&changed), None);
}
