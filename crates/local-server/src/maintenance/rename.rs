//! 双路径审阅始终同时呈现固定效果与当前观测；前者不能由后者推导。
use super::{MaintenanceError, presentation};
use crate::file_operation_audit::{RenameShape, valid_rename_effects};
use serde_json::{Value, json};
use studiolo_vault_broker::{
    FileEffect, FileOperationKind, FileOperationPhase, FileOperationStatus, FilePresence,
    MaintenanceCurrent, MaintenanceTarget, MaintenanceTargetKind,
};

pub(super) fn valid(target: &MaintenanceTarget) -> bool {
    let (Some(secondary), Some(operation)) = (&target.secondary, &target.file_operation) else {
        return false;
    };
    let FileOperationKind::Rename { expected, to } = &operation.kind else {
        return false;
    };
    if target.file_operation_id.is_none()
        || secondary.path != *to
        || target.previous_revision != Some(*expected)
        || target.target_revision.is_some()
        || secondary.previous_revision.is_some()
    {
        return false;
    }
    let (kind, shape) = match operation.phase {
        FileOperationPhase::Preparing | FileOperationPhase::Prepared => {
            (MaintenanceTargetKind::Prepared, RenameShape::NotApplied)
        }
        FileOperationPhase::TargetInstallIssued => (
            MaintenanceTargetKind::Installing,
            RenameShape::TargetUnknown,
        ),
        FileOperationPhase::TargetInstalled => (
            MaintenanceTargetKind::Installing,
            RenameShape::TargetInstalled,
        ),
        FileOperationPhase::SourceDetachIssued => (
            MaintenanceTargetKind::Installing,
            RenameShape::SourceUnknown,
        ),
        _ => return false,
    };
    if target.kind != kind
        || !valid_rename_effects(&target.path, to, *expected, &operation.effects, shape)
    {
        return false;
    }
    let prepared = kind == MaintenanceTargetKind::Prepared;
    if prepared != (target.current == MaintenanceCurrent::NotObserved)
        || prepared != (secondary.current == MaintenanceCurrent::NotObserved)
    {
        return false;
    }
    if operation.phase == FileOperationPhase::Preparing {
        return secondary.target_revision.is_none()
            && target.current == MaintenanceCurrent::NotObserved
            && secondary.current == MaintenanceCurrent::NotObserved
            && !target.proposed_available
            && !secondary.proposed_available;
    }
    let Some(revision) = secondary.target_revision else {
        return false;
    };
    if revision == *expected
        || (operation.phase == FileOperationPhase::Prepared
            && (!target.proposed_available || !secondary.proposed_available))
    {
        return false;
    }
    // 已安装目标的历史版本必须与完整目标提案一致；不要求当前仍是该版本。
    !matches!(&operation.effects[0], FileEffect::Confirmed {
        after: FilePresence::Present { revision: actual, .. }, ..
    } if actual != &revision)
}

pub(super) fn extend(
    target: &MaintenanceTarget,
    status: Option<&FileOperationStatus>,
    note: &mut Value,
) -> Result<(), MaintenanceError> {
    if !valid(target) {
        return Err(MaintenanceError::Broker);
    }
    let secondary = target.secondary.as_ref().ok_or(MaintenanceError::Broker)?;
    let operation = target
        .file_operation
        .as_ref()
        .ok_or(MaintenanceError::Broker)?;
    let status = status.ok_or(MaintenanceError::Broker)?;
    // 固定意图与当前元数据必须属于同一请求；不重新扫描可能已经不安全的目标。
    if status.preview.kind != operation.kind {
        return Err(MaintenanceError::Broker);
    }
    let availability = match operation.phase {
        FileOperationPhase::Preparing => {
            "准备尚未完整形成提案；本次未观察两侧历史和暂存，proposed_available=false 不表示对象缺失。"
        }
        FileOperationPhase::Prepared => {
            "准备时已完整形成提案；本次未再观察两侧历史和暂存，proposed_available=true 不证明此刻仍可读取。"
        }
        _ => {
            "proposed_available 是本次完整观察所得的提案可用性；不表示已经安装，也不保证后续仍可读取。"
        }
    };
    note["rename"] = json!({
        "source": {
            "path": target.path, "previous_revision": target.previous_revision,
            "target_revision": target.target_revision, "current": target.current,
            "current_observability": presentation::current(target.current),
            "proposed_available": target.proposed_available
        },
        "destination": {
            "path": secondary.path, "previous_revision": secondary.previous_revision,
            "target_revision": secondary.target_revision, "current": secondary.current,
            "current_observability": presentation::current(secondary.current),
            "proposed_available": secondary.proposed_available
        },
        "reviewed_phase": operation.phase, "known_effects": operation.effects,
        "proposed_available_note": availability,
        "effects_note": "固定效果按目标、源排列，是执行时已持久的事实；当前版本、存在或缺失不改变这些事实。",
        "preservation_note": "保全须涵盖源与目标当前可观察对象、源原件、目标提案、源隔离件和目标暂存；不能观察不等于缺失。保全不补跑摘源或撤回已安装目标。"
    });
    Ok(())
}
