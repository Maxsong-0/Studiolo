use super::{
    super::{MaintenanceError, inputs::Inputs, options::Command, workflow::execute},
    fixture::Fixture,
    workflow::lost_restore,
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};
use studiolo_vault_broker::{MaintenanceActor, MaintenanceDecision};

struct Failing {
    ledger: SqliteAuditLedger,
    phase: AuditPhase,
}
impl AuditLedger for Failing {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        if event.action() == "writer.maintenance" && event.phase() == self.phase {
            return Err(AuditError::Unavailable);
        }
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}

#[test]
fn failed_maintenance_audit_never_claims_completion_and_reopen_can_flush() {
    for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
        let fixture = Fixture::new(true, true);
        let mut options = fixture.options(Command::CancelRestoreIntent);
        let mut inputs = Inputs::open(&options).unwrap();
        options.request = Some(lost_restore(&mut inputs));
        inputs.audit = Some(Box::new(Failing {
            ledger: SqliteAuditLedger::open_existing(&fixture.root.join("audit")).unwrap(),
            phase,
        }));
        assert!(matches!(
            execute(
                &mut inputs,
                &options,
                MaintenanceActor::current().unwrap(),
                |_, _| Ok(()),
                || Ok(())
            ),
            Err(MaintenanceError::Audit)
        ));
        let state = inputs.broker.maintenance_inspect(64).unwrap();
        assert!(state.writer.unresolved);
        assert_eq!(
            state.pending.unwrap().execution_started,
            phase == AuditPhase::Outcome
        );
        assert_eq!(
            inputs.broker.pending_recorded_restores(64).unwrap().len(),
            usize::from(phase == AuditPhase::Intent)
        );
        drop(inputs);
        let mut inputs = Inputs::open(&options).unwrap();
        options.command = Command::AuditFlush;
        options.request = None;
        let report = execute(
            &mut inputs,
            &options,
            MaintenanceActor::current().unwrap(),
            |_, _| Ok(()),
            || Ok(()),
        )
        .unwrap();
        if phase == AuditPhase::Intent {
            assert_eq!(report["status"]["pending"]["execution_started"], false);
            assert_eq!(
                inputs.broker.pending_recorded_restores(64).unwrap().len(),
                1
            );
        } else {
            assert!(report["status"]["pending"].is_null());
            assert!(!inputs.broker.write_status().unwrap().unresolved);
        }
    }
}

#[test]
fn cancellation_of_unstarted_maintenance_survives_original_target_drift() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelMaintenance);
    let mut inputs = Inputs::open(&options).unwrap();
    let selector = lost_restore(&mut inputs);
    drop(
        inputs
            .broker
            .prepare_maintenance(
                &selector,
                MaintenanceDecision::CancelRestoreIntent,
                MaintenanceActor::current().unwrap(),
            )
            .unwrap(),
    );
    options.maintenance = Some(
        inputs
            .broker
            .maintenance_inspect(64)
            .unwrap()
            .pending
            .unwrap()
            .maintenance_id,
    );
    drop(inputs);
    std::fs::write(fixture.root.join("vault/note.md"), b"EXTERNAL_NEW_BODY").unwrap();
    let mut inputs = Inputs::open(&options).unwrap();
    let report = execute(
        &mut inputs,
        &options,
        MaintenanceActor::current().unwrap(),
        |_, _| Ok(()),
        || Ok(()),
    )
    .unwrap();
    assert_eq!(report["receipt"]["event"]["outcome"], "cancelled");
    assert_eq!(
        std::fs::read(fixture.root.join("vault/note.md")).unwrap(),
        b"EXTERNAL_NEW_BODY"
    );
    assert_eq!(
        inputs.broker.pending_recorded_restores(64).unwrap().len(),
        1
    );
}

struct WrongReceipt(studiolo_audit_ledger::MemoryAuditLedger);
impl AuditLedger for WrongReceipt {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        self.0.append(AuditEvent::new(
            event.event_id().clone(),
            event.phase(),
            "wrong.action",
            event.payload().clone(),
        ))
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.0.query(filter)
    }
}

#[test]
fn valid_hash_for_a_different_event_is_not_an_intent_ack() {
    let fixture = Fixture::new(true, true);
    let mut options = fixture.options(Command::CancelRestoreIntent);
    let mut inputs = Inputs::open(&options).unwrap();
    options.request = Some(lost_restore(&mut inputs));
    inputs.audit = Some(Box::new(WrongReceipt(
        studiolo_audit_ledger::MemoryAuditLedger::new(),
    )));
    assert!(matches!(
        execute(
            &mut inputs,
            &options,
            MaintenanceActor::current().unwrap(),
            |_, _| Ok(()),
            || Ok(())
        ),
        Err(MaintenanceError::Audit)
    ));
    let pending = inputs
        .broker
        .maintenance_inspect(64)
        .unwrap()
        .pending
        .unwrap();
    assert!(!pending.intent_confirmed);
    assert!(!pending.execution_started);
    assert_eq!(
        inputs.broker.pending_recorded_restores(64).unwrap().len(),
        1
    );
}
