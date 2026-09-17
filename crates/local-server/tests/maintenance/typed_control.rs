use super::{
    super::{MaintenanceError, inputs::Inputs, options::Command, presentation, workflow::execute},
    fixture::Fixture,
    typed_fixture::{lost_delete, selector},
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, SqliteAuditLedger};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{MaintenanceActor, MaintenanceDecision};

#[test]
fn typed_confirmation_failure_does_not_even_ack_original_intent() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelPrepared);
    let mut inputs = Inputs::open(&options).unwrap();
    lost_delete(&mut inputs);
    options.request = Some(selector(&inputs));
    assert!(matches!(
        execute(
            &mut inputs,
            &options,
            MaintenanceActor::current().unwrap(),
            |_, _| Err(MaintenanceError::Confirmation),
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
fn cancel_maintenance_only_closes_its_own_typed_slot_even_when_target_is_unsafe() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelMaintenance);
    let mut inputs = Inputs::open(&options).unwrap();
    lost_delete(&mut inputs);
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
    std::os::unix::fs::symlink(
        "/NONEXISTENT_TYPED_FIXTURE",
        fixture.root.join("vault/note.md"),
    )
    .unwrap();
    let report = execute(
        &mut inputs,
        &options,
        MaintenanceActor::current().unwrap(),
        |_, _| Ok(()),
        || Ok(()),
    )
    .unwrap();
    assert_eq!(report["receipt"]["event"]["outcome"], "cancelled");
    assert!(report["receipt"]["event"]["resolution"].is_null());
    assert!(inputs.broker.maintenance_pending().unwrap().is_none());
    assert!(inputs.broker.file_operation_status().unwrap().is_some());
    assert!(inputs.broker.write_status().unwrap().unresolved);
    let ledger = SqliteAuditLedger::open_existing(&fixture.root.join("audit")).unwrap();
    assert_eq!(
        ledger
            .query(&AuditFilter::default().with_action("writer.maintenance"))
            .unwrap()
            .len(),
        2
    );
    let records = ledger.query(&AuditFilter::default()).unwrap();
    let order: Vec<_> = records
        .iter()
        .map(|row| (row.event().action(), row.event().phase()))
        .collect();
    assert_eq!(
        order,
        vec![
            ("vault.mutation", AuditPhase::Intent),
            ("writer.maintenance", AuditPhase::Intent),
            ("writer.maintenance", AuditPhase::Outcome),
        ],
        "维护自身取消不得先于原typed意图，也不得造出原操作Outcome"
    );
    assert!(
        std::fs::symlink_metadata(fixture.root.join("vault/note.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn typed_review_never_borrows_the_kind_of_an_unrelated_operation() {
    let fixture = Fixture::new(true, true);
    let options = fixture.options(Command::Inspect);
    let mut inputs = Inputs::open(&options).unwrap();
    lost_delete(&mut inputs);
    let target = inputs
        .broker
        .maintenance_inspect(64)
        .unwrap()
        .targets
        .remove(0);
    let mut status = inputs.broker.file_operation_status().unwrap().unwrap();
    assert!(presentation::target(&target, Some(&status)).is_ok());
    assert!(presentation::target(&target, None).is_err());
    status.operation_id = "f".repeat(64);
    assert!(presentation::target(&target, Some(&status)).is_err());
    status.operation_id = status.preview.operation_id.clone();
    status.preview.path = VaultPath::parse("other.md").unwrap();
    assert!(presentation::target(&target, Some(&status)).is_err());
}
