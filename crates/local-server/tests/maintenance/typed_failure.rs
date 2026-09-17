use super::{
    super::{MaintenanceError, inputs::Inputs, options::Command, workflow::execute},
    fixture::Fixture,
    typed_fixture::{lost_delete, metadata, selector},
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};
use studiolo_vault_broker::{ContentAuditPhase, FileOperationOutcome, MaintenanceActor};

struct Failing {
    ledger: SqliteAuditLedger,
    phase: AuditPhase,
    maintenance: bool,
}
impl AuditLedger for Failing {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let target = if self.maintenance {
            "writer.maintenance"
        } else {
            "vault.mutation"
        };
        if event.action() == target && event.phase() == self.phase {
            return Err(AuditError::Unavailable);
        }
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}

#[test]
fn each_real_ack_failure_retains_exact_pending_phase_and_reopen_flush_closes_only_known_facts() {
    for maintenance in [false, true] {
        for phase in [AuditPhase::Intent, AuditPhase::Outcome] {
            let fixture = Fixture::new(true, true);
            let mut options = fixture.options(Command::CancelPrepared);
            let mut inputs = Inputs::open(&options).unwrap();
            let request = lost_delete(&mut inputs);
            options.request = Some(selector(&inputs));
            inputs.audit = Some(Box::new(Failing {
                ledger: SqliteAuditLedger::open_existing(&fixture.root.join("audit")).unwrap(),
                phase,
                maintenance,
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
            let pending = inputs.broker.maintenance_pending().unwrap();
            if phase == AuditPhase::Intent {
                if maintenance {
                    assert!(!pending.unwrap().execution_started);
                } else {
                    assert!(pending.is_none());
                }
            } else {
                let pending = pending.unwrap();
                assert!(pending.execution_started);
                assert!(pending.outcome.is_some());
                let originals = inputs.broker.pending_file_operation_audit(2).unwrap();
                assert_eq!(originals.len(), usize::from(!maintenance));
                if !maintenance {
                    assert_eq!(originals[0].event().phase, ContentAuditPhase::Outcome);
                }
                assert!(!pending.outcome_confirmed);
            }
            drop(inputs);
            let mut inputs = Inputs::open(&options).unwrap();
            options.command = Command::AuditFlush;
            options.request = None;
            execute(
                &mut inputs,
                &options,
                MaintenanceActor::current().unwrap(),
                |_, _| Ok(()),
                || Ok(()),
            )
            .unwrap();
            let original = inputs
                .broker
                .file_operation_replay(&request, &metadata())
                .unwrap()
                .unwrap();
            if phase == AuditPhase::Intent {
                assert!(original.receipt.is_none());
                assert!(inputs.broker.write_status().unwrap().unresolved);
                assert!(original.audit_pending);
                if maintenance {
                    let pending = inputs.broker.maintenance_pending().unwrap().unwrap();
                    assert!(pending.intent_confirmed);
                    assert!(!pending.execution_started);
                }
            } else {
                assert!(!original.audit_pending);
                assert_eq!(
                    original.receipt.unwrap().outcome,
                    FileOperationOutcome::Cancelled
                );
                assert!(inputs.broker.maintenance_pending().unwrap().is_none());
                assert!(!inputs.broker.write_status().unwrap().unresolved);
            }
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
            if phase == AuditPhase::Outcome {
                assert_eq!(records.len(), 4);
                let original = records
                    .iter()
                    .position(|row| {
                        row.event().action() == "vault.mutation"
                            && row.event().phase() == AuditPhase::Outcome
                    })
                    .unwrap();
                let maintained = records
                    .iter()
                    .position(|row| {
                        row.event().action() == "writer.maintenance"
                            && row.event().phase() == AuditPhase::Outcome
                    })
                    .unwrap();
                assert!(original < maintained);
            }
        }
    }
}

#[test]
fn rejected_original_needs_two_flush_rounds_but_no_maintenance_or_visible_change() {
    let fixture = Fixture::new(true, true);
    let options = fixture.options(Command::AuditFlush);
    let mut inputs = Inputs::open(&options).unwrap();
    super::typed_fixture::initialize(&mut inputs);
    let request = super::typed_fixture::request(&inputs);
    std::fs::write(fixture.root.join("vault/note.md"), b"new external bytes").unwrap();
    drop(
        inputs
            .broker
            .prepare_file_operation(request.clone(), metadata())
            .unwrap(),
    );
    assert!(
        inputs
            .broker
            .maintenance_inspect(64)
            .unwrap()
            .targets
            .is_empty()
    );
    execute(
        &mut inputs,
        &options,
        MaintenanceActor::current().unwrap(),
        |_, _| Ok(()),
        || Ok(()),
    )
    .unwrap();
    let replay = inputs
        .broker
        .file_operation_replay(&request, &metadata())
        .unwrap()
        .unwrap();
    assert_eq!(
        replay.receipt.unwrap().outcome,
        FileOperationOutcome::Rejected
    );
    assert!(!replay.audit_pending);
    assert!(inputs.broker.maintenance_pending().unwrap().is_none());
    let records = inputs
        .audit
        .as_ref()
        .unwrap()
        .query(&AuditFilter::default())
        .unwrap();
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|row| row.event().action() == "vault.mutation")
    );
    assert_eq!(
        std::fs::read(fixture.root.join("vault/note.md")).unwrap(),
        b"new external bytes"
    );
}
