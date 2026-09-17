use super::{
    super::{MaintenanceError, inputs::Inputs, options::Command, workflow::execute},
    fixture::Fixture,
    rename_fixture::lost_rename,
    typed_fixture::selector,
};
use studiolo_audit_ledger::{AuditFilter, AuditPhase};
use studiolo_vault_broker::{MaintenanceActor, MaintenanceDecision};

#[test]
fn rejecting_dual_path_review_does_not_ack_or_create_a_maintenance_slot() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelPrepared);
    let mut inputs = Inputs::open(&options).unwrap();
    lost_rename(&mut inputs);
    options.request = Some(selector(&inputs));
    assert!(matches!(
        execute(
            &mut inputs,
            &options,
            MaintenanceActor::current().unwrap(),
            |value, _| {
                assert_eq!(
                    value["explanation"]["rename"]["destination"]["path"],
                    "renamed.md"
                );
                Err(MaintenanceError::Confirmation)
            },
            || Ok(())
        ),
        Err(MaintenanceError::Confirmation)
    ));
    assert!(inputs.broker.maintenance_pending().unwrap().is_none());
    assert_eq!(
        inputs.broker.pending_file_operation_audit(2).unwrap().len(),
        1
    );
    assert!(
        inputs
            .audit
            .as_ref()
            .unwrap()
            .query(&AuditFilter::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn cancelling_rename_maintenance_uses_fixed_dual_review_even_when_both_paths_are_unsafe() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelMaintenance);
    let mut inputs = Inputs::open(&options).unwrap();
    lost_rename(&mut inputs);
    let review = selector(&inputs);
    drop(
        inputs
            .broker
            .prepare_maintenance(
                &review,
                MaintenanceDecision::CancelPrepared,
                MaintenanceActor::current().unwrap(),
            )
            .unwrap(),
    );
    options.maintenance = Some(
        inputs
            .broker
            .maintenance_pending()
            .unwrap()
            .unwrap()
            .maintenance_id,
    );
    std::fs::remove_file(fixture.root.join("vault/note.md")).unwrap();
    for name in ["note.md", "renamed.md"] {
        std::os::unix::fs::symlink(
            "/NONEXISTENT_RENAME_FIXTURE",
            fixture.root.join("vault").join(name),
        )
        .unwrap();
    }
    let report = execute(
        &mut inputs,
        &options,
        MaintenanceActor::current().unwrap(),
        |value, _| {
            assert_eq!(value["explanation"]["rename"]["reviewed_phase"], "prepared");
            Ok(())
        },
        || Ok(()),
    )
    .unwrap();
    assert_eq!(report["receipt"]["event"]["outcome"], "cancelled");
    assert!(report["receipt"]["event"]["resolution"].is_null());
    assert!(inputs.broker.maintenance_pending().unwrap().is_none());
    assert!(inputs.broker.file_operation_status().unwrap().is_some());
    assert!(inputs.broker.write_status().unwrap().unresolved);
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
            ("writer.maintenance", AuditPhase::Outcome),
        ]
    );
    for name in ["note.md", "renamed.md"] {
        assert!(
            std::fs::symlink_metadata(fixture.root.join("vault").join(name))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }
}
