use super::{
    super::{MaintenanceError, inputs::Inputs, options::Command, workflow::execute},
    fixture::Fixture,
    rename_fixture::lost_rename,
    typed_fixture::{metadata, selector},
};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};
use studiolo_vault_broker::{FileOperationOutcome, MaintenanceActor};

struct FailOutcome {
    ledger: SqliteAuditLedger,
    maintenance: bool,
}
impl AuditLedger for FailOutcome {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let target = if self.maintenance {
            "writer.maintenance"
        } else {
            "vault.mutation"
        };
        if event.action() == target && event.phase() == AuditPhase::Outcome {
            return Err(AuditError::Unavailable);
        }
        self.ledger.append(event)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}

#[test]
fn either_outcome_ack_failure_blocks_release_and_reopen_flush_only_closes_fixed_events() {
    for maintenance in [false, true] {
        let fixture = Fixture::new(true, true);
        let mut options = fixture.options(Command::CancelPrepared);
        let mut inputs = Inputs::open(&options).unwrap();
        let request = lost_rename(&mut inputs);
        options.request = Some(selector(&inputs));
        inputs.audit = Some(Box::new(FailOutcome {
            ledger: SqliteAuditLedger::open_existing(&fixture.root.join("audit")).unwrap(),
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
        let pending = inputs.broker.maintenance_pending().unwrap().unwrap();
        assert!(pending.execution_started && pending.intent_confirmed);
        assert!(pending.outcome.is_some() && !pending.outcome_confirmed);
        assert!(inputs.broker.write_status().unwrap().unresolved);
        let before = inputs
            .audit
            .as_ref()
            .unwrap()
            .query(&AuditFilter::default())
            .unwrap();
        assert_eq!(before.len(), if maintenance { 3 } else { 2 });
        drop(inputs);
        options.command = Command::AuditFlush;
        options.request = None;
        let mut inputs = Inputs::open(&options).unwrap();
        execute(
            &mut inputs,
            &options,
            MaintenanceActor::current().unwrap(),
            |_, _| Ok(()),
            || Ok(()),
        )
        .unwrap();
        assert!(inputs.broker.maintenance_pending().unwrap().is_none());
        assert!(!inputs.broker.write_status().unwrap().unresolved);
        let original = inputs
            .broker
            .file_operation_replay(&request, &metadata())
            .unwrap()
            .unwrap();
        assert!(!original.audit_pending);
        assert_eq!(
            original.receipt.unwrap().outcome,
            FileOperationOutcome::Cancelled
        );
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
        assert_eq!(
            std::fs::read(fixture.root.join("vault/note.md")).unwrap(),
            "# 私有虚构正文\n".as_bytes()
        );
        assert!(!fixture.root.join("vault/renamed.md").exists());
    }
}
