use super::{
    super::{flush, inputs::Inputs, options::Command, workflow::execute},
    fixture::Fixture,
    typed_fixture::{initialize, lost_delete, metadata, request, selector},
};
use studiolo_audit_ledger::{AuditFilter, AuditPhase};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{
    FileOperationKind, FileOperationOutcome, FileOperationRequest, MaintenanceActor,
};

#[test]
fn typed_prepared_cancel_preserves_body_and_records_four_real_acks_in_order() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelPrepared);
    let mut inputs = Inputs::open(&options).unwrap();
    let request = lost_delete(&mut inputs);
    options.request = Some(selector(&inputs));
    drop(inputs);
    let mut inputs = Inputs::open(&options).unwrap();
    let report = execute(
        &mut inputs,
        &options,
        MaintenanceActor::current().unwrap(),
        |value, expected| {
            assert!(expected.starts_with("cancel-prepared "));
            assert_eq!(value["target"]["target_revision"], serde_json::Value::Null);
            assert_eq!(
                value["explanation"]["original_operation"]["kind"]["kind"],
                "delete"
            );
            assert!(
                value["explanation"]["current_observability"]
                    .as_str()
                    .unwrap()
                    .starts_with("未观察")
            );
            Ok(())
        },
        || Ok(()),
    )
    .unwrap();
    assert_eq!(report["receipt"]["event"]["schema_version"], 2);
    assert_eq!(report["receipt"]["event"]["resolution"], "cancelled");
    assert!(report["status"]["pending"].is_null());
    assert!(!inputs.broker.write_status().unwrap().unresolved);
    let replay = inputs
        .broker
        .file_operation_replay(&request, &metadata())
        .unwrap()
        .unwrap();
    assert!(!replay.audit_pending);
    assert_eq!(
        replay.receipt.unwrap().outcome,
        FileOperationOutcome::Cancelled
    );
    assert_eq!(
        std::fs::read(fixture.root.join("vault/note.md")).unwrap(),
        "# 私有虚构正文\n".as_bytes()
    );
    let records = inputs
        .audit
        .as_ref()
        .unwrap()
        .query(&AuditFilter::default())
        .unwrap();
    assert_eq!(records.len(), 4);
    let actual: Vec<_> = records
        .iter()
        .map(|row| (row.event().action(), row.event().phase()))
        .collect();
    assert_eq!(
        actual,
        vec![
            ("vault.mutation", AuditPhase::Intent),
            ("writer.maintenance", AuditPhase::Intent),
            ("vault.mutation", AuditPhase::Outcome),
            ("writer.maintenance", AuditPhase::Outcome)
        ]
    );
    for row in [&records[0], &records[2]] {
        let payload = row.event().payload().as_value();
        assert_eq!(payload["schema_version"], 2);
        assert_eq!(payload["file_operation"]["schema_version"], 1);
        assert_eq!(
            payload["file_operation"]["actor"]["subject"],
            "original-human"
        );
    }
    for forbidden in [
        "PRIVATE_TYPED_REQUEST_KEY",
        "私有虚构正文",
        "NOT_A_SECRET_PATH",
    ] {
        assert!(!report.to_string().contains(forbidden));
    }
}

#[test]
fn restore_tombstone_prepared_is_identified_from_kind_and_cancel_keeps_missing() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelPrepared);
    let mut inputs = Inputs::open(&options).unwrap();
    initialize(&mut inputs);
    let original = request(&inputs);
    let ticket = inputs
        .broker
        .prepare_file_operation(original, metadata())
        .unwrap();
    flush::file_operations(&mut inputs, &|| Ok(())).unwrap();
    let receipt = inputs.broker.commit_file_operation(&ticket).unwrap();
    flush::file_operations(&mut inputs, &|| Ok(())).unwrap();
    let request = FileOperationRequest::RestoreTombstone {
        path: VaultPath::parse("note.md").unwrap(),
        tombstone_id: receipt.tombstone_id.unwrap(),
        idempotency_key: "PRIVATE_RESTORE_KEY".into(),
    };
    drop(
        inputs
            .broker
            .prepare_file_operation(request.clone(), metadata())
            .unwrap(),
    );
    let status = inputs.broker.file_operation_status().unwrap().unwrap();
    assert!(matches!(
        status.preview.kind,
        FileOperationKind::RestoreTombstone { .. }
    ));
    options.request = Some(selector(&inputs));
    execute(
        &mut inputs,
        &options,
        MaintenanceActor::current().unwrap(),
        |value, _| {
            assert_eq!(
                value["explanation"]["original_operation"]["kind"]["kind"],
                "restore_tombstone"
            );
            Ok(())
        },
        || Ok(()),
    )
    .unwrap();
    assert!(!fixture.root.join("vault/note.md").exists());
    assert_eq!(
        inputs
            .broker
            .file_operation_replay(&request, &metadata())
            .unwrap()
            .unwrap()
            .receipt
            .unwrap()
            .outcome,
        FileOperationOutcome::Cancelled
    );
}
