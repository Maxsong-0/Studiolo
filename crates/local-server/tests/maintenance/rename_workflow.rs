use super::{
    super::{flush, inputs::Inputs, options::Command, workflow::execute},
    fixture::Fixture,
    rename_fixture::lost_rename,
    typed_fixture::{metadata, selector},
};
use studiolo_audit_ledger::{AuditFilter, AuditPhase};
use studiolo_vault_broker::{FileEffect, FileOperationOutcome, FilePresence, MaintenanceActor};

#[test]
fn rename_prepared_reopen_cancel_has_four_real_acks_and_keeps_both_paths_unchanged() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelPrepared);
    let mut inputs = Inputs::open(&options).unwrap();
    let request = lost_rename(&mut inputs);
    options.request = Some(selector(&inputs));
    drop(inputs);
    let mut inputs = Inputs::open(&options).unwrap();
    let report = execute(
        &mut inputs,
        &options,
        MaintenanceActor::current().unwrap(),
        |value, expected| {
            assert!(expected.starts_with("cancel-prepared "));
            assert_eq!(value["target"]["path"], "note.md");
            assert_eq!(value["target"]["secondary"]["path"], "renamed.md");
            assert!(value["target"]["target_revision"].is_null());
            assert!(!value["target"]["secondary"]["target_revision"].is_null());
            let note = &value["explanation"]["rename"];
            assert_eq!(note["source"]["proposed_available"], true);
            assert_eq!(note["destination"]["proposed_available"], true);
            assert!(
                note["proposed_available_note"]
                    .as_str()
                    .unwrap()
                    .contains("本次未再观察")
            );
            assert_eq!(note["known_effects"][0]["path"], "renamed.md");
            assert_eq!(note["known_effects"][1]["path"], "note.md");
            Ok(())
        },
        || Ok(()),
    )
    .unwrap();
    assert_eq!(report["receipt"]["event"]["schema_version"], 3);
    assert_eq!(report["receipt"]["event"]["resolution"], "cancelled");
    assert!(report["status"]["pending"].is_null());
    assert!(!inputs.broker.write_status().unwrap().unresolved);
    let replay = inputs
        .broker
        .file_operation_replay(&request, &metadata())
        .unwrap()
        .unwrap();
    assert!(!replay.audit_pending);
    let receipt = replay.receipt.unwrap();
    assert_eq!(receipt.outcome, FileOperationOutcome::Cancelled);
    assert!(
        receipt
            .effects
            .iter()
            .all(|effect| matches!(effect, FileEffect::NotApplied { .. }))
    );
    assert_eq!(
        std::fs::read(fixture.root.join("vault/note.md")).unwrap(),
        "# 私有虚构正文\n".as_bytes()
    );
    assert!(!fixture.root.join("vault/renamed.md").exists());
    let rows = inputs
        .audit
        .as_ref()
        .unwrap()
        .query(&AuditFilter::default())
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| (row.event().action(), row.event().phase()))
            .collect::<Vec<_>>(),
        vec![
            ("vault.mutation", AuditPhase::Intent),
            ("writer.maintenance", AuditPhase::Intent),
            ("vault.mutation", AuditPhase::Outcome),
            ("writer.maintenance", AuditPhase::Outcome),
        ]
    );
    for row in [&rows[0], &rows[2]] {
        let payload = row.event().payload().as_value();
        assert_eq!(payload["schema_version"], 3);
        assert_eq!(payload["file_operation"]["schema_version"], 2);
        assert_eq!(
            payload["file_operation"]["actor"]["subject"],
            "original-human"
        );
    }
    for row in [&rows[1], &rows[3]] {
        assert_eq!(row.event().payload().as_value()["schema_version"], 3);
        assert_eq!(
            row.event().payload().as_value()["actor"]["kind"],
            "local_os"
        );
    }
    for forbidden in [
        "PRIVATE_RENAME_REQUEST_KEY",
        "私有虚构正文",
        "NOT_A_SECRET_PATH",
    ] {
        assert!(!report.to_string().contains(forbidden));
        assert!(rows.iter().all(|row| {
            !row.event()
                .payload()
                .as_value()
                .to_string()
                .contains(forbidden)
        }));
    }
}

#[test]
fn real_rename_committed_effects_append_once_and_replay_the_same_parent_receipt() {
    let fixture = Fixture::new(true, true);
    let options = fixture.options(Command::AuditFlush);
    let mut inputs = Inputs::open(&options).unwrap();
    let request = lost_rename(&mut inputs);
    let ticket = inputs
        .broker
        .prepare_file_operation(request.clone(), metadata())
        .unwrap();
    flush::file_operations(&mut inputs, &|| Ok(())).unwrap();
    let receipt = inputs.broker.commit_file_operation(&ticket).unwrap();
    assert_eq!(receipt.outcome, FileOperationOutcome::Committed);
    flush::file_operations(&mut inputs, &|| Ok(())).unwrap();
    let replay = inputs
        .broker
        .file_operation_replay(&request, &metadata())
        .unwrap()
        .unwrap();
    assert!(!replay.audit_pending);
    assert_eq!(replay.receipt.as_ref(), Some(&receipt));
    let [
        FileEffect::Confirmed {
            after:
                FilePresence::Present {
                    revision: to,
                    sha256: target_raw,
                    bytes: target_bytes,
                },
            ..
        },
        FileEffect::Confirmed {
            before:
                FilePresence::Present {
                    revision: from,
                    sha256: source_raw,
                    bytes: source_bytes,
                },
            ..
        },
    ] = receipt.effects.as_slice()
    else {
        panic!("缺少实际双效果");
    };
    assert_ne!(to, from);
    assert_eq!((target_raw, target_bytes), (source_raw, source_bytes));
    assert_eq!(
        std::fs::read(fixture.root.join("vault/renamed.md")).unwrap(),
        "# 私有虚构正文\n".as_bytes()
    );
    assert!(!fixture.root.join("vault/note.md").exists());
    assert_eq!(receipt.tombstone_id.as_ref(), Some(&receipt.operation_id));
    let rows = inputs
        .audit
        .as_ref()
        .unwrap()
        .query(&AuditFilter::default())
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[1].event().payload().as_value()["file_operation"]["effects"],
        serde_json::to_value(&receipt.effects).unwrap()
    );
}
