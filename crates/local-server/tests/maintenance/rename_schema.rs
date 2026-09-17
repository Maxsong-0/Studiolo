use super::{
    super::{audit, presentation},
    rename_fixture::{event, status},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, MemoryAuditLedger};
use studiolo_core::{Revision, VaultPath};
use studiolo_vault_broker::{
    ContentAuditPhase, FileEffect, FileOperationPhase, MaintenanceCurrent, MaintenanceOutcome,
    MaintenanceUnavailableReason, ReconciliationOutcome,
};

#[test]
fn five_review_phases_preserve_complete_schema_three_and_outcome_identity() {
    for phase in [
        FileOperationPhase::Preparing,
        FileOperationPhase::Prepared,
        FileOperationPhase::TargetInstallIssued,
        FileOperationPhase::TargetInstalled,
        FileOperationPhase::SourceDetachIssued,
    ] {
        let mut value = event(phase);
        let note = presentation::target(&value.target, Some(&status(&value.target))).unwrap();
        let explanation = note["rename"]["proposed_available_note"].as_str().unwrap();
        match phase {
            FileOperationPhase::Preparing => assert!(explanation.contains("本次未观察")),
            FileOperationPhase::Prepared => {
                assert!(explanation.contains("准备时已完整形成提案"));
                assert!(explanation.contains("本次未再观察"));
                assert!(explanation.contains("不证明此刻仍可读取"));
            }
            _ => assert!(explanation.contains("本次完整观察所得")),
        }
        let ledger = MemoryAuditLedger::new();
        let ack = audit::append(&ledger, &value).unwrap();
        assert_eq!(audit::append(&ledger, &value).unwrap(), ack);
        let original_target = serde_json::to_value(&value.target).unwrap();
        value.phase = ContentAuditPhase::Outcome;
        value.outcome = Some(MaintenanceOutcome::Applied);
        let prepared = matches!(
            phase,
            FileOperationPhase::Preparing | FileOperationPhase::Prepared
        );
        value.resolution = Some(if prepared {
            ReconciliationOutcome::Cancelled
        } else {
            ReconciliationOutcome::Indeterminate
        });
        value.evidence_id = (!prepared).then(|| "7".repeat(64));
        audit::append(&ledger, &value).unwrap();
        let records = ledger.query(&AuditFilter::default()).unwrap();
        assert_eq!(records.len(), 2);
        for row in records {
            let payload = row.event().payload().as_value();
            assert_eq!(payload["schema_version"], 3);
            assert_eq!(payload["target"], original_target);
            assert_eq!(
                payload["actor"],
                serde_json::json!({"kind":"local_os","uid":501,"gid":20})
            );
        }
    }
}

#[test]
fn current_observation_cannot_replace_known_effects_or_hide_the_second_path() {
    let mut value = event(FileOperationPhase::TargetInstalled);
    value.target.current = MaintenanceCurrent::Missing;
    value.target.secondary.as_mut().unwrap().current = MaintenanceCurrent::Unobservable {
        reason: MaintenanceUnavailableReason::UnsafeOrUnstable,
    };
    let fixed = value
        .target
        .file_operation
        .as_ref()
        .unwrap()
        .effects
        .clone();
    let note = presentation::target(&value.target, Some(&status(&value.target))).unwrap();
    assert_eq!(
        note["rename"]["known_effects"],
        serde_json::to_value(&fixed).unwrap()
    );
    assert_eq!(note["rename"]["source"]["current"]["kind"], "missing");
    assert_eq!(
        note["rename"]["destination"]["current"]["kind"],
        "unobservable"
    );
    assert_eq!(note["rename"]["known_effects"][0]["kind"], "confirmed");
    assert!(
        note["rename"]["destination"]["current_observability"]
            .as_str()
            .unwrap()
            .contains("不是缺失")
    );
    audit::append(&MemoryAuditLedger::new(), &value).unwrap();
    let mut wrong = status(&value.target);
    if let studiolo_vault_broker::FileOperationKind::Rename { to, .. } = &mut wrong.preview.kind {
        *to = VaultPath::parse("unrelated.md").unwrap();
    }
    assert!(presentation::target(&value.target, Some(&wrong)).is_err());
}

#[test]
fn mixed_versions_missing_roles_and_fabricated_effects_never_reach_audit() {
    let mut invalid = Vec::new();
    for version in [None, Some(1), Some(2), Some(4)] {
        let mut value = event(FileOperationPhase::Prepared);
        value.schema_version = version;
        invalid.push(value);
    }
    let mut value = event(FileOperationPhase::Prepared);
    value.target.secondary = None;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Prepared);
    value.target.file_operation = None;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Prepared);
    value.target.file_operation_id = None;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Prepared);
    value.target.sequence = None;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Prepared);
    value.target.previous_revision = None;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Prepared);
    value.target.target_revision = value.target.previous_revision;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Prepared);
    value.target.secondary.as_mut().unwrap().target_revision = None;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Prepared);
    value.target.secondary.as_mut().unwrap().proposed_available = false;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Preparing);
    value.target.current = MaintenanceCurrent::Missing;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Prepared);
    value.target.secondary.as_mut().unwrap().current = MaintenanceCurrent::Missing;
    invalid.push(value);
    let mut value = event(FileOperationPhase::TargetInstallIssued);
    value.target.current = MaintenanceCurrent::NotObserved;
    invalid.push(value);
    let mut value = event(FileOperationPhase::Prepared);
    value.target.file_operation.as_mut().unwrap().phase = FileOperationPhase::SourceDetached;
    invalid.push(value);
    let mut value = event(FileOperationPhase::TargetInstalled);
    value
        .target
        .file_operation
        .as_mut()
        .unwrap()
        .effects
        .swap(0, 1);
    invalid.push(value);
    let mut value = event(FileOperationPhase::TargetInstalled);
    value.target.secondary.as_mut().unwrap().target_revision = Some(Revision::compute(
        value.vault_uuid,
        &value.target.path,
        b"other",
    ));
    invalid.push(value);
    let mut value = event(FileOperationPhase::TargetInstalled);
    value.target.file_operation.as_mut().unwrap().effects[0] = FileEffect::Unknown {
        path: value.target.secondary.as_ref().unwrap().path.clone(),
    };
    invalid.push(value);
    for value in invalid {
        let ledger = MemoryAuditLedger::new();
        assert!(audit::append(&ledger, &value).is_err());
        assert!(ledger.query(&AuditFilter::default()).unwrap().is_empty());
    }
}
