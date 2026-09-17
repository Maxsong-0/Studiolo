//! 纯元数据夹具不声称执行过半步；实际故障由 Broker 私有契约验证。
use super::super::inputs::Inputs;
use studiolo_core::{Revision, VaultPath, VaultUuid};
use studiolo_vault_broker::{
    ContentAuditPhase, FileEffect, FileOperationKind, FileOperationPhase, FileOperationPreview,
    FileOperationRequest, FileOperationStatus, FilePresence, MaintenanceActor,
    MaintenanceAuditEvent, MaintenanceCurrent, MaintenanceDecision, MaintenanceFileOperation,
    MaintenanceSecondaryTarget, MaintenanceTarget, MaintenanceTargetKind,
};

pub(super) fn lost_rename(inputs: &mut Inputs) -> FileOperationRequest {
    super::typed_fixture::initialize(inputs);
    inputs.broker.enable_file_rename().unwrap();
    let from = VaultPath::parse("note.md").unwrap();
    let source = inputs.broker.read_file(&from, 4096).unwrap();
    let request = FileOperationRequest::Rename {
        from,
        expected: source.revision,
        to: VaultPath::parse("renamed.md").unwrap(),
        idempotency_key: "PRIVATE_RENAME_REQUEST_KEY".into(),
    };
    drop(
        inputs
            .broker
            .prepare_file_operation(request.clone(), super::typed_fixture::metadata())
            .unwrap(),
    );
    request
}

pub(super) fn event(phase: FileOperationPhase) -> MaintenanceAuditEvent {
    let vault_uuid = VaultUuid::new();
    let from = VaultPath::parse("note.md").unwrap();
    let to = VaultPath::parse("renamed.md").unwrap();
    let expected = Revision::compute(vault_uuid, &from, b"old");
    let target_revision = Revision::compute(vault_uuid, &to, b"old");
    let preparing = phase == FileOperationPhase::Preparing;
    let prepared = matches!(
        phase,
        FileOperationPhase::Preparing | FileOperationPhase::Prepared
    );
    let current = if prepared {
        MaintenanceCurrent::NotObserved
    } else {
        MaintenanceCurrent::Missing
    };
    let target_effect = match phase {
        FileOperationPhase::Preparing | FileOperationPhase::Prepared => {
            FileEffect::NotApplied { path: to.clone() }
        }
        FileOperationPhase::TargetInstallIssued => FileEffect::Unknown { path: to.clone() },
        _ => FileEffect::Confirmed {
            path: to.clone(),
            before: FilePresence::Missing,
            after: FilePresence::Present {
                revision: target_revision,
                sha256: "6".repeat(64),
                bytes: 3,
            },
        },
    };
    let source_effect = if phase == FileOperationPhase::SourceDetachIssued {
        FileEffect::Unknown { path: from.clone() }
    } else {
        FileEffect::NotApplied { path: from.clone() }
    };
    MaintenanceAuditEvent {
        schema_version: Some(3),
        event_id: "1".repeat(64),
        phase: ContentAuditPhase::Intent,
        vault_uuid,
        actor: MaintenanceActor::LocalOs { uid: 501, gid: 20 },
        decision: if prepared {
            MaintenanceDecision::CancelPrepared
        } else {
            MaintenanceDecision::PreserveInstalling
        },
        requested_at_ms: None,
        target: MaintenanceTarget {
            review_id: "2".repeat(64),
            request_fingerprint: "3".repeat(64),
            path: from,
            kind: if prepared {
                MaintenanceTargetKind::Prepared
            } else {
                MaintenanceTargetKind::Installing
            },
            sequence: Some(1),
            previous_revision: Some(expected),
            target_revision: None,
            file_operation_id: Some("4".repeat(64)),
            current,
            proposed_available: !preparing,
            secondary: Some(MaintenanceSecondaryTarget {
                path: to.clone(),
                previous_revision: None,
                target_revision: (!preparing).then_some(target_revision),
                current,
                proposed_available: !preparing,
            }),
            file_operation: Some(MaintenanceFileOperation {
                kind: FileOperationKind::Rename { expected, to },
                phase,
                effects: vec![target_effect, source_effect],
            }),
        },
        outcome: None,
        resolution: None,
        evidence_id: None,
    }
}

pub(super) fn status(target: &MaintenanceTarget) -> FileOperationStatus {
    let operation = target.file_operation.as_ref().unwrap();
    let id = target.file_operation_id.clone().unwrap();
    FileOperationStatus {
        operation_id: id.clone(),
        phase: operation.phase,
        audit_pending: true,
        preview: FileOperationPreview {
            operation_id: id,
            path: target.path.clone(),
            kind: operation.kind.clone(),
            observed_before: FilePresence::Present {
                revision: target.previous_revision.unwrap(),
                sha256: "6".repeat(64),
                bytes: 3,
            },
            fixed_outcome: None,
        },
        receipt: None,
    }
}
