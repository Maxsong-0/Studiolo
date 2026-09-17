use super::{
    super::{MaintenanceError, inputs::Inputs, options::Command, workflow::execute},
    fixture::Fixture,
};
use serde_json::Value;
use studiolo_audit_ledger::{AuditFilter, AuditLedger, SqliteAuditLedger};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{
    MaintenanceActor, Mutation, MutationActor, MutationKind, MutationMetadata,
};

pub(super) fn lost_restore(inputs: &mut Inputs) -> String {
    let path = VaultPath::parse("note.md").unwrap();
    let original = inputs.broker.read_file(&path, 4096).unwrap();
    let actor = MutationActor::Human {
        issuer: "https://original.example.com".into(),
        subject: "original-human".into(),
    };
    let next = inputs
        .broker
        .mutate_recorded(
            Mutation::replace(
                path.clone(),
                b"CURRENT_PRIVATE_BODY".to_vec(),
                original.revision,
                "prior-save",
            ),
            MutationMetadata::new(actor.clone(), MutationKind::Replace).unwrap(),
        )
        .unwrap();
    drop(
        inputs
            .broker
            .prepare_recorded_restore(
                Mutation::replace(
                    path,
                    original.content,
                    next.revision,
                    "LOST_PRIVATE_REQUEST_KEY",
                ),
                MutationMetadata::new(
                    actor,
                    MutationKind::Restore {
                        source_revision: original.revision,
                    },
                )
                .unwrap(),
            )
            .unwrap(),
    );
    inputs.broker.maintenance_inspect(64).unwrap().targets[0]
        .review_id
        .clone()
}

fn accept(_: &Value, _: &str) -> Result<(), MaintenanceError> {
    Ok(())
}
fn check() -> Result<(), MaintenanceError> {
    Ok(())
}

#[test]
fn lost_restore_is_cancelled_with_original_actor_and_separate_os_audit() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelRestoreIntent);
    let mut inputs = Inputs::open(&options).unwrap();
    options.request = Some(lost_restore(&mut inputs));
    drop(inputs);
    let mut inputs = Inputs::open(&options).unwrap();
    let result = execute(
        &mut inputs,
        &options,
        MaintenanceActor::current().unwrap(),
        accept,
        check,
    )
    .unwrap();
    assert!(result["status"]["pending"].is_null());
    assert_eq!(result["receipt"]["event"]["outcome"], "applied");
    assert_eq!(result["receipt"]["event"]["resolution"], "cancelled");
    assert_eq!(result["restore_unconfirmed"], false);
    assert_eq!(result["content_audit_pending"], 0);
    assert_eq!(result["status"]["targets_status"], "not_observed");
    assert!(result["status"]["targets"].is_null());
    let encoded = result.to_string();
    for forbidden in [
        "CURRENT_PRIVATE_BODY",
        "LOST_PRIVATE_REQUEST_KEY",
        "NOT_A_SECRET_PATH",
        "NOT_THE_LOCAL_OPERATOR",
        "私有虚构正文",
    ] {
        assert!(!encoded.contains(forbidden));
    }
    assert_eq!(
        std::fs::read(fixture.root.join("vault/note.md")).unwrap(),
        b"CURRENT_PRIVATE_BODY"
    );
    let ledger = SqliteAuditLedger::open_existing(&fixture.root.join("audit")).unwrap();
    let maintenance = ledger
        .query(&AuditFilter::default().with_action("writer.maintenance"))
        .unwrap();
    assert_eq!(maintenance.len(), 2);
    let original = ledger
        .query(&AuditFilter::default().with_action("recovery.page_restore"))
        .unwrap();
    assert_eq!(original.len(), 2);
    let body = serde_json::to_string(original[1].event()).unwrap();
    assert!(body.contains("original-human"));
    assert!(!body.contains("local_os"));
}

#[test]
fn failed_confirmation_and_config_drift_do_not_prepare_maintenance() {
    for drift in [false, true] {
        let fixture = Fixture::new(true, true);
        let mut options = fixture.options(Command::CancelRestoreIntent);
        let mut inputs = Inputs::open(&options).unwrap();
        options.request = Some(lost_restore(&mut inputs));
        let result = execute(
            &mut inputs,
            &options,
            MaintenanceActor::current().unwrap(),
            |_, _| {
                if drift {
                    let mut bytes = std::fs::read(&fixture.config).unwrap();
                    bytes.push(b' ');
                    std::fs::write(&fixture.config, bytes).unwrap();
                    Ok(())
                } else {
                    Err(MaintenanceError::Confirmation)
                }
            },
            check,
        );
        assert!(matches!(
            result,
            Err(MaintenanceError::Confirmation | MaintenanceError::Configuration)
        ));
        assert!(
            inputs
                .broker
                .maintenance_inspect(64)
                .unwrap()
                .pending
                .is_none()
        );
        assert_eq!(
            inputs.broker.pending_recorded_restores(64).unwrap().len(),
            1
        );
    }
}

#[test]
fn missing_audit_refuses_before_confirmation_and_preserves_the_restore() {
    let fixture = Fixture::new(false, true);
    let mut options = fixture.options(Command::CancelRestoreIntent);
    let mut inputs = Inputs::open(&options).unwrap();
    options.request = Some(lost_restore(&mut inputs));
    assert!(matches!(
        execute(
            &mut inputs,
            &options,
            MaintenanceActor::current().unwrap(),
            |_, _| panic!("审计缺失不进入确认"),
            check
        ),
        Err(MaintenanceError::Audit)
    ));
    assert!(
        inputs
            .broker
            .maintenance_inspect(64)
            .unwrap()
            .pending
            .is_none()
    );
    assert_eq!(
        std::fs::read_dir(fixture.root.join("audit"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn audit_flush_only_confirms_the_intent_and_cancel_maintenance_leaves_original_pending() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::AuditFlush);
    let mut inputs = Inputs::open(&options).unwrap();
    let selector = lost_restore(&mut inputs);
    let actor = MaintenanceActor::current().unwrap();
    drop(
        inputs
            .broker
            .prepare_maintenance(
                &selector,
                studiolo_vault_broker::MaintenanceDecision::CancelRestoreIntent,
                actor.clone(),
            )
            .unwrap(),
    );
    drop(inputs);
    let mut inputs = Inputs::open(&options).unwrap();
    let flushed = execute(&mut inputs, &options, actor.clone(), accept, check).unwrap();
    assert_eq!(flushed["status"]["pending"]["intent_confirmed"], true);
    assert_eq!(flushed["status"]["pending"]["execution_started"], false);
    assert!(flushed["status"]["pending"]["outcome"].is_null());
    assert_eq!(
        inputs.broker.pending_recorded_restores(64).unwrap().len(),
        1
    );
    options.command = Command::CancelMaintenance;
    options.maintenance = Some(
        flushed["status"]["pending"]["maintenance_id"]
            .as_str()
            .unwrap()
            .to_owned(),
    );
    let cancelled = execute(&mut inputs, &options, actor, accept, check).unwrap();
    assert_eq!(cancelled["receipt"]["event"]["outcome"], "cancelled");
    assert!(cancelled["receipt"]["event"]["resolution"].is_null());
    assert!(cancelled["status"]["pending"].is_null());
    assert_eq!(
        inputs.broker.pending_recorded_restores(64).unwrap().len(),
        1
    );
}
