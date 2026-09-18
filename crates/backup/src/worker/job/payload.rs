//! 备份审计载荷：固定字段、固定事件键，永不放 stdout、token 或原配置。

use super::super::recovery_point::BackupTargetKind;
use super::super::restic::UnverifiedReason;
use super::super::state::{FailureReason, JobKey, RemoteVerification};
use serde_json::json;

/// Intent 与 Outcome 共用的稳定事件键。
pub(super) fn event_id_text(key: &JobKey) -> String {
    format!(
        "backup.snapshot:{}:{}",
        key.sequence,
        key.checkpoint_id.as_str()
    )
}

/// Intent 在任何真实副作用之前持久。
pub(super) fn intent(key: &JobKey, target: BackupTargetKind) -> serde_json::Value {
    json!({
        "job_sequence": key.sequence.get(),
        "checkpoint_id": key.checkpoint_id.as_str(),
        "target_kind": target,
        "phase_note": "intent_persisted_before_any_side_effect",
    })
}

/// 成功的 Outcome：补记时必须逐字节一致，所以这里只依赖作业键与校验结果。
pub(super) fn outcome_success(
    key: &JobKey,
    verification: &RemoteVerification,
) -> serde_json::Value {
    json!({
        "job_sequence": key.sequence.get(),
        "checkpoint_id": key.checkpoint_id.as_str(),
        "result": "succeeded",
        "objects_complete": verification.objects_complete,
        "checksum_verified": verification.checksum_verified,
        "paired_checkpoint_reference": verification.paired_checkpoint_reference,
        // Outcome 发生在 checkpoint 截止之后，不包含在本次 Audit 数据库快照内。
        "recorded_after_checkpoint_cutoff": true,
    })
}

/// 明确失败的 Outcome。
pub(super) fn outcome_failed(key: &JobKey, reason: FailureReason) -> serde_json::Value {
    json!({
        "job_sequence": key.sequence.get(),
        "checkpoint_id": key.checkpoint_id.as_str(),
        "result": "failed",
        "reason": reason,
        "recorded_after_checkpoint_cutoff": true,
    })
}

/// 明确未验证的 Outcome；既不是成功也不是失败。
pub(super) fn outcome_unverified(key: &JobKey, reason: UnverifiedReason) -> serde_json::Value {
    json!({
        "job_sequence": key.sequence.get(),
        "checkpoint_id": key.checkpoint_id.as_str(),
        "result": "unverified",
        "reason": reason,
        "recorded_after_checkpoint_cutoff": true,
    })
}
