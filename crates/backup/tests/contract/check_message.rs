//! 仓库抽查的两种消息：派检查只能带同目标的抽查原意图；上报只能导出唯一一条结果事件，
//! 回执必须就是那条事件。三种消息互相解不出来。
use super::queue_message::check_assigned;
use serde_json::json;
use std::num::NonZeroU64;
use studiolo_audit_ledger::{AuditLedger, MemoryAuditLedger};
use studiolo_backup::worker::{
    audit::{
        BackupAuditRequest, BackupQueueRequest, QueueAnswer, QueueCommand, RepositoryCheckReport,
    },
    job::backup_intent,
    recovery_point::BackupTargetKind,
    repository_check::{
        CheckAssignment, CheckBucket, CheckKey, CheckOutcome, CheckPurpose, CheckRecorder,
        IncompleteReason, RepositoryDigest, check_abandoned, check_intent, check_outcome,
    },
    restic::RepositoryRef,
    state::JobKey,
};
use studiolo_core::checkpoint::CheckpointId;

const TARGET: BackupTargetKind = BackupTargetKind::DedicatedAppendOnlyTarget;

fn check_key() -> CheckKey {
    CheckKey {
        sequence: NonZeroU64::new(3).unwrap(),
        bucket: CheckBucket::new(5).unwrap(),
    }
}

fn digest() -> RepositoryDigest {
    RepositoryDigest::of(&RepositoryRef::parse("rest:https://backup.example.test/repo/").unwrap())
}

fn report(outcome: CheckOutcome) -> RepositoryCheckReport {
    RepositoryCheckReport::new(TARGET, check_key(), outcome, digest()).unwrap()
}

#[test]
fn an_assignment_carries_only_a_check_intent_for_the_same_target() {
    let check = BackupQueueRequest::new(TARGET, QueueCommand::RepositoryCheck);
    let ledger = MemoryAuditLedger::new();
    // 别的目标类别的抽查意图不行。
    let other = check_assigned(&ledger, BackupTargetKind::LocalTemporaryRepository);
    assert!(check.encode_reply(&other).is_err());
    // 备份作业的原意图、抽查的结果都不是抽查意图。
    let job = JobKey {
        sequence: NonZeroU64::new(9).unwrap(),
        checkpoint_id: CheckpointId::parse("check-wire").unwrap(),
    };
    let backup = ledger.append(backup_intent(&job, TARGET).unwrap()).unwrap();
    let outcome = ledger
        .append(check_abandoned(&check_key()).unwrap())
        .unwrap();
    for record in [backup, outcome] {
        assert!(
            check
                .encode_reply(&QueueAnswer::CheckAssigned(Box::new(record)))
                .is_err()
        );
    }
    // 线上把「没到」改成带回执的派检查，拒绝。
    let not_due: serde_json::Value =
        serde_json::from_slice(&check.encode_reply(&QueueAnswer::NotDue).unwrap()).unwrap();
    let mut forged = not_due;
    forged["answer"] = json!("check_assigned");
    assert!(
        check
            .decode_reply(&serde_json::to_vec(&forged).unwrap())
            .is_err()
    );
    // 回执被改过一个字节，完整性校验不过。
    let QueueAnswer::CheckAssigned(record) = check_assigned(&MemoryAuditLedger::new(), TARGET)
    else {
        unreachable!()
    };
    let mut value = serde_json::to_value(record.as_ref()).unwrap();
    value["event"]["payload"]["bucket"] = json!(6);
    let tampered = serde_json::from_value(value).unwrap();
    assert!(
        check
            .encode_reply(&QueueAnswer::CheckAssigned(Box::new(tampered)))
            .is_err()
    );
}

#[test]
fn a_report_roundtrips_and_its_reply_must_be_exactly_its_own_outcome() {
    let ledger = MemoryAuditLedger::new();
    ledger
        .append(
            check_intent(
                &CheckAssignment {
                    key: check_key(),
                    purpose: CheckPurpose::Retry,
                },
                TARGET,
            )
            .unwrap(),
        )
        .unwrap();
    for outcome in [
        CheckOutcome::clean(),
        CheckOutcome::problem(2, 1).unwrap(),
        CheckOutcome::incomplete(IncompleteReason::Preempted),
    ] {
        let request = report(outcome);
        let bytes = request.encode().unwrap();
        let decoded = RepositoryCheckReport::decode(&bytes).unwrap();
        assert_eq!(decoded.key(), &check_key());
        assert_eq!(decoded.outcome(), &outcome);
        assert_eq!(decoded.repository(), &digest());
        assert_eq!(decoded.target(), TARGET);
        let event = decoded.event().unwrap();
        assert_eq!(
            event,
            check_outcome(&check_key(), &outcome, &CheckRecorder::Worker(digest())).unwrap()
        );
        let record = MemoryAuditLedger::new().append(event).unwrap();
        let reply = decoded.encode_reply(&record).unwrap();
        assert_eq!(request.decode_reply(&reply).unwrap(), record);
        // 答复绑定请求：拿别的上报的答复来，摘要对不上。
        let other = report(CheckOutcome::incomplete(IncompleteReason::Deadline));
        assert!(other.decode_reply(&reply).is_err());
    }
    // 回执不是这份上报导出的事件（结果不同、或是原意图），拒绝。
    let request = report(CheckOutcome::clean());
    let different = MemoryAuditLedger::new()
        .append(
            check_outcome(
                &check_key(),
                &CheckOutcome::problem(1, 0).unwrap(),
                &CheckRecorder::Worker(digest()),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(request.encode_reply(&different).is_err());
    let abandoned = MemoryAuditLedger::new()
        .append(check_abandoned(&check_key()).unwrap())
        .unwrap();
    assert!(request.encode_reply(&abandoned).is_err());
}

#[test]
fn a_worker_cannot_report_an_abandoned_check_or_a_malformed_message() {
    assert!(
        RepositoryCheckReport::new(
            TARGET,
            check_key(),
            CheckOutcome::incomplete(IncompleteReason::Abandoned),
            digest()
        )
        .is_err()
    );
    let bytes = report(CheckOutcome::clean()).encode().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let mut abandoned = value.clone();
    abandoned["outcome"]["result"] = json!("incomplete");
    abandoned["outcome"]["reason"] = json!("abandoned");
    let mut extra = value.clone();
    extra["note"] = json!("x");
    let mut bucket = value.clone();
    bucket["check"]["bucket"] = json!(31);
    let mut inconsistent = value.clone();
    inconsistent["outcome"]["errors"] = json!(1);
    let mut short = value.clone();
    short["repository"] = json!("ab");
    for forged in [abandoned, extra, bucket, inconsistent, short] {
        assert!(
            RepositoryCheckReport::decode(&serde_json::to_vec(&forged).unwrap()).is_err(),
            "{forged}"
        );
    }
    let mut spaced = bytes.clone();
    spaced.push(b' ');
    assert!(
        RepositoryCheckReport::decode(&spaced).is_err(),
        "非规范编码"
    );
    assert!(RepositoryCheckReport::decode(b"").is_err());
}

#[test]
fn the_three_message_kinds_cannot_be_decoded_as_each_other() {
    let report = report(CheckOutcome::clean()).encode().unwrap();
    let queue = BackupQueueRequest::new(TARGET, QueueCommand::RepositoryCheck)
        .encode()
        .unwrap();
    let keyed = BackupAuditRequest::admission(
        JobKey {
            sequence: NonZeroU64::new(9).unwrap(),
            checkpoint_id: CheckpointId::parse("check-wire").unwrap(),
        },
        TARGET,
    )
    .encode()
    .unwrap();
    assert!(BackupQueueRequest::decode(&report).is_err());
    assert!(BackupAuditRequest::decode(&report).is_err());
    assert!(RepositoryCheckReport::decode(&queue).is_err());
    assert!(RepositoryCheckReport::decode(&keyed).is_err());
}
