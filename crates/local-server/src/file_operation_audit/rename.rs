//! 复合效果只有目标、源两项；验证历史元数据，不根据当前路径重建成功。
use super::{hex, presence_valid};
use studiolo_audit_ledger::AuditError;
use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{
    ContentAuditPhase, FileEffect, FileOperationAuditEvent, FileOperationKind,
    FileOperationOutcome, FilePresence,
};

#[derive(Clone, Copy)]
pub(crate) enum RenameShape {
    NotApplied,
    TargetUnknown,
    TargetInstalled,
    SourceUnknown,
    Committed,
}

pub(super) fn validate(value: &FileOperationAuditEvent) -> Result<(), AuditError> {
    let FileOperationKind::Rename { expected, to } = &value.kind else {
        return Err(AuditError::Serialization);
    };
    if to == &value.path {
        return Err(AuditError::Serialization);
    }
    if value.phase == ContentAuditPhase::Intent {
        return if value.outcome.is_none()
            && value.effects.is_empty()
            && value.tombstone_id.is_none()
            && value.evidence_id.is_none()
        {
            Ok(())
        } else {
            Err(AuditError::Serialization)
        };
    }
    let accepts = |shape| valid_rename_effects(&value.path, to, *expected, &value.effects, shape);
    let valid = match value.outcome {
        Some(FileOperationOutcome::Committed) => {
            accepts(RenameShape::Committed)
                && value.tombstone_id.as_ref() == Some(&value.operation_id)
                && value.evidence_id.is_none()
        }
        Some(FileOperationOutcome::Rejected | FileOperationOutcome::Cancelled) => {
            accepts(RenameShape::NotApplied)
                && value.tombstone_id.is_none()
                && value.evidence_id.is_none()
        }
        Some(FileOperationOutcome::Indeterminate) => {
            (accepts(RenameShape::TargetUnknown)
                || accepts(RenameShape::TargetInstalled)
                || accepts(RenameShape::SourceUnknown))
                && value.tombstone_id.is_none()
                && value.evidence_id.as_deref().is_some_and(hex)
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(AuditError::Serialization)
    }
}

/// 原事件与维护展示共用同一效果闭集；调用者仍须核对版本、阶段和完整回执。
pub(crate) fn valid_rename_effects(
    from: &VaultPath,
    to: &VaultPath,
    expected: Revision,
    effects: &[FileEffect],
    shape: RenameShape,
) -> bool {
    let [target, source] = effects else {
        return false;
    };
    if from == to {
        return false;
    }
    match shape {
        RenameShape::NotApplied => not_applied(target, to) && not_applied(source, from),
        RenameShape::TargetUnknown => unknown(target, to) && not_applied(source, from),
        RenameShape::TargetInstalled => {
            installed(target, to, expected).is_some() && not_applied(source, from)
        }
        RenameShape::SourceUnknown => {
            installed(target, to, expected).is_some() && unknown(source, from)
        }
        RenameShape::Committed => {
            let Some(FilePresence::Present { sha256, bytes, .. }) = installed(target, to, expected)
            else {
                return false;
            };
            matches!(source, FileEffect::Confirmed {
                path,
                before: FilePresence::Present { revision, sha256: raw, bytes: size },
                after: FilePresence::Missing,
            } if path == from && *revision == expected && raw == sha256 && size == bytes)
        }
    }
}

fn installed<'a>(
    effect: &'a FileEffect,
    to: &VaultPath,
    expected: Revision,
) -> Option<&'a FilePresence> {
    match effect {
        FileEffect::Confirmed {
            path,
            before: FilePresence::Missing,
            after: present @ FilePresence::Present { revision, .. },
        } if path == to && *revision != expected && presence_valid(present) => Some(present),
        _ => None,
    }
}

fn not_applied(effect: &FileEffect, expected: &VaultPath) -> bool {
    matches!(effect, FileEffect::NotApplied { path } if path == expected)
}

fn unknown(effect: &FileEffect, expected: &VaultPath) -> bool {
    matches!(effect, FileEffect::Unknown { path } if path == expected)
}
