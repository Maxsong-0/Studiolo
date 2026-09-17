//! typed 原事件的唯一 Audit 包装；只核对固定元数据，不访问文件或授予主体权限。

mod rename;

pub(crate) use rename::{RenameShape, valid_rename_effects};

use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditEventId, AuditLedger, AuditPhase, SanitizedPayload,
};
use studiolo_vault_broker::{
    ContentAuditPhase, FileEffect, FileOperationAuditEvent, FileOperationKind,
    FileOperationOutcome, FilePresence, MutationActor,
};

/// 只有完整原事件和完整真实回执均通过核对，才返回可交还 Broker 的 ACK。
pub(crate) fn append(
    ledger: &dyn AuditLedger,
    value: &FileOperationAuditEvent,
) -> Result<(u64, String), AuditError> {
    let event = event(value)?;
    let record = ledger.append(event.clone())?;
    if record.event() != &event || record.verify_integrity().is_err() {
        return Err(AuditError::CorruptStore);
    }
    Ok((record.offset(), record.record_hash().to_hex()))
}

fn event(value: &FileOperationAuditEvent) -> Result<AuditEvent, AuditError> {
    validate(value)?;
    let action = match value.kind {
        FileOperationKind::Delete { .. } | FileOperationKind::Rename { .. } => "vault.mutation",
        FileOperationKind::RestoreTombstone { .. } => "recovery.page_restore",
    };
    let phase = match value.phase {
        ContentAuditPhase::Intent => AuditPhase::Intent,
        ContentAuditPhase::Outcome => AuditPhase::Outcome,
    };
    // 旧 nested 1/outer 2 保持原样；只有 Rename 使用 nested 2/outer 3。
    let payload =
        serde_json::json!({"schema_version":value.schema_version + 1,"file_operation":value});
    Ok(AuditEvent::new(
        AuditEventId::parse(&value.event_id)?,
        phase,
        action,
        SanitizedPayload::try_from(payload)?,
    ))
}

fn validate(value: &FileOperationAuditEvent) -> Result<(), AuditError> {
    let version = if matches!(value.kind, FileOperationKind::Rename { .. }) {
        2
    } else {
        1
    };
    if value.schema_version != version
        || !hex(&value.event_id)
        || !hex(&value.operation_id)
        || !hex(&value.request_fingerprint)
        || !hex(value.scope_hash.as_str())
        || value.scope_epoch == 0
        || value.requested_at_ms.is_some_and(|time| time < 0)
        || !actor_valid(&value.actor)
    {
        return Err(AuditError::Serialization);
    }
    if matches!(value.kind, FileOperationKind::Rename { .. }) {
        return rename::validate(value);
    }
    if let FileOperationKind::RestoreTombstone { tombstone_id } = &value.kind
        && !hex(tombstone_id)
    {
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
    let [effect] = value.effects.as_slice() else {
        return Err(AuditError::Serialization);
    };
    let valid = match (value.outcome, effect) {
        (
            Some(FileOperationOutcome::Committed),
            FileEffect::Confirmed {
                path,
                before,
                after,
            },
        ) => {
            path == &value.path
                && presence_valid(before)
                && presence_valid(after)
                && value.evidence_id.is_none()
                && match (&value.kind, before, after) {
                    (
                        FileOperationKind::Delete { expected },
                        FilePresence::Present { revision, .. },
                        FilePresence::Missing,
                    ) => {
                        expected == revision
                            && value.tombstone_id.as_ref() == Some(&value.operation_id)
                    }
                    (
                        FileOperationKind::RestoreTombstone { .. },
                        FilePresence::Missing,
                        FilePresence::Present { .. },
                    ) => value.tombstone_id.is_none(),
                    _ => false,
                }
        }
        (
            Some(FileOperationOutcome::Rejected | FileOperationOutcome::Cancelled),
            FileEffect::NotApplied { path },
        ) => path == &value.path && value.tombstone_id.is_none() && value.evidence_id.is_none(),
        (Some(FileOperationOutcome::Indeterminate), FileEffect::Unknown { path }) => {
            path == &value.path
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

fn presence_valid(value: &FilePresence) -> bool {
    match value {
        FilePresence::Missing => true,
        FilePresence::Present { sha256, bytes, .. } => hex(sha256) && *bytes <= 268_435_456,
    }
}

fn actor_valid(actor: &MutationActor) -> bool {
    match actor {
        MutationActor::Obsidian => true,
        MutationActor::Human { issuer, subject } => {
            issuer.starts_with("https://") && text(issuer, 512) && text(subject, 256)
        }
    }
}

fn text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
#[path = "../tests/file_operation_audit/mod.rs"]
mod tests;
