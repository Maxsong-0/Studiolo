//! 抽查的固定审计事件：意图在跑 restic 之前落账，结果在之后。上报、补记、读回走同一套写法，
//! 读回时按字段重新生成一遍再整条比较，只对上动作名不算。
use super::{
    CHECK_BUCKETS, CheckAssignment, CheckBucket, CheckKey, CheckOutcome, CheckRecorder,
    IncompleteReason, REPOSITORY_CHECK_ACTION,
};
use crate::worker::recovery_point::BackupTargetKind;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::num::NonZeroU64;
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};

/// 意图和结果共用的事件键；和备份作业的 `backup.snapshot:…` 不重叠。
fn event_id(sequence: NonZeroU64) -> Result<AuditEventId, AuditError> {
    AuditEventId::parse(format!("{REPOSITORY_CHECK_ACTION}:{sequence}"))
}

fn event(
    sequence: NonZeroU64,
    phase: AuditPhase,
    payload: Value,
) -> Result<AuditEvent, AuditError> {
    Ok(AuditEvent::new(
        event_id(sequence)?,
        phase,
        REPOSITORY_CHECK_ACTION,
        SanitizedPayload::try_from(payload)?,
    ))
}

/// 意图：生产者派检查时先落账，worker 拿到它才跑 restic。
pub fn check_intent(
    assignment: &CheckAssignment,
    target: BackupTargetKind,
) -> Result<AuditEvent, AuditError> {
    event(
        assignment.key.sequence,
        AuditPhase::Intent,
        json!({
            "check_sequence": assignment.key.sequence.get(),
            "bucket": assignment.key.bucket,
            "buckets": CHECK_BUCKETS,
            "purpose": assignment.purpose,
            "target_kind": target,
            "phase_note": "intent_persisted_before_check",
        }),
    )
}

/// 结果。worker 不能写「中途没了」，生产者只能写「中途没了」：两边各自的唯一形状。
pub fn check_outcome(
    key: &CheckKey,
    outcome: &CheckOutcome,
    recorder: &CheckRecorder,
) -> Result<AuditEvent, AuditError> {
    let abandoned = outcome.reason() == Some(IncompleteReason::Abandoned);
    let (repository, recorded_by) = match recorder {
        CheckRecorder::Worker(digest) if !abandoned => {
            (Value::String(digest.as_str().to_owned()), "worker")
        }
        CheckRecorder::Producer if abandoned => (Value::Null, "producer"),
        _ => return Err(AuditError::Serialization),
    };
    event(
        key.sequence,
        AuditPhase::Outcome,
        json!({
            "check_sequence": key.sequence.get(),
            "bucket": key.bucket,
            "buckets": CHECK_BUCKETS,
            "result": outcome.result(),
            "reason": outcome.reason(),
            "errors": outcome.errors(),
            "broken_packs": outcome.broken_packs(),
            "repository": repository,
            "recorded_by": recorded_by,
        }),
    )
}

/// 生产者给中途没了的检查补的「没做完」。
pub fn check_abandoned(key: &CheckKey) -> Result<AuditEvent, AuditError> {
    check_outcome(
        key,
        &CheckOutcome::incomplete(IncompleteReason::Abandoned),
        &CheckRecorder::Producer,
    )
}

/// 账本里的一条抽查事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckEvent {
    /// 意图。
    Intent {
        /// 派下的检查。
        assignment: CheckAssignment,
        /// 目标类别。
        target: BackupTargetKind,
    },
    /// 结果。
    Outcome {
        /// 检查键。
        key: CheckKey,
        /// 结果。
        outcome: CheckOutcome,
        /// 谁写的。
        recorder: CheckRecorder,
    },
}

/// 读回一条抽查事件；不是抽查事件、或和固定写法逐字段对不上，都是 `None`。
#[must_use]
pub fn read_check_event(event: &AuditEvent) -> Option<CheckEvent> {
    if event.action() != REPOSITORY_CHECK_ACTION {
        return None;
    }
    let value = event.payload().as_value();
    let key = CheckKey {
        sequence: NonZeroU64::new(value.get("check_sequence")?.as_u64()?)?,
        bucket: field::<CheckBucket>(value, "bucket")?,
    };
    let (parsed, expected) = match event.phase() {
        AuditPhase::Intent => {
            let assignment = CheckAssignment {
                key,
                purpose: field(value, "purpose")?,
            };
            let target = field(value, "target_kind")?;
            (
                CheckEvent::Intent { assignment, target },
                check_intent(&assignment, target).ok()?,
            )
        }
        AuditPhase::Outcome => {
            let outcome: CheckOutcome = serde_json::from_value(json!({
                "result": value.get("result")?,
                "reason": value.get("reason")?,
                "errors": value.get("errors")?,
                "broken_packs": value.get("broken_packs")?,
            }))
            .ok()?;
            let recorder = match value.get("recorded_by")?.as_str()? {
                "worker" => CheckRecorder::Worker(field(value, "repository")?),
                "producer" => CheckRecorder::Producer,
                _ => return None,
            };
            let expected = check_outcome(&key, &outcome, &recorder).ok()?;
            (
                CheckEvent::Outcome {
                    key,
                    outcome,
                    recorder,
                },
                expected,
            )
        }
    };
    (event == &expected).then_some(parsed)
}

fn field<T: DeserializeOwned>(value: &Value, name: &str) -> Option<T> {
    serde_json::from_value(value.get(name)?.clone()).ok()
}
