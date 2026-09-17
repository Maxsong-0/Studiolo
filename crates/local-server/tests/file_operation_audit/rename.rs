use super::*;

pub(super) fn rename_intent() -> FileOperationAuditEvent {
    let mut value = intent(false);
    let FileOperationKind::Delete { expected } = value.kind else {
        unreachable!()
    };
    value.schema_version = 2;
    value.kind = FileOperationKind::Rename {
        expected,
        to: VaultPath::parse("renamed.md").unwrap(),
    };
    value
}

pub(super) fn rename_committed() -> FileOperationAuditEvent {
    committed_from(rename_intent())
}

fn committed_from(mut value: FileOperationAuditEvent) -> FileOperationAuditEvent {
    let FileOperationKind::Rename { expected, to } = &value.kind else {
        unreachable!()
    };
    value.effects = vec![
        FileEffect::Confirmed {
            path: to.clone(),
            before: FilePresence::Missing,
            after: FilePresence::Present {
                revision: Revision::compute(value.vault_uuid, to, b"old"),
                sha256: "6".repeat(64),
                bytes: 3,
            },
        },
        FileEffect::Confirmed {
            path: value.path.clone(),
            before: FilePresence::Present {
                revision: *expected,
                sha256: "6".repeat(64),
                bytes: 3,
            },
            after: FilePresence::Missing,
        },
    ];
    value.phase = ContentAuditPhase::Outcome;
    value.outcome = Some(FileOperationOutcome::Committed);
    value.tombstone_id = Some(value.operation_id.clone());
    value
}

#[test]
fn rename_two_phase_real_records_use_outer_three_and_preserve_original_actor() {
    let value = rename_intent();
    let outcome = committed_from(value.clone());
    let ledger = MemoryAuditLedger::new();
    for fixed in [&value, &outcome] {
        let ack = append(&ledger, fixed).unwrap();
        assert_eq!(append(&ledger, fixed).unwrap(), ack);
    }
    let records = ledger.query(&AuditFilter::default()).unwrap();
    assert_eq!(records.len(), 2);
    for (row, fixed) in records.iter().zip([&value, &outcome]) {
        row.verify_integrity().unwrap();
        assert_eq!(row.event().action(), "vault.mutation");
        assert_eq!(
            row.event().payload().as_value(),
            &serde_json::json!({
                "schema_version":3,"file_operation":fixed
            })
        );
        assert_eq!(
            row.event().payload().as_value()["file_operation"]["actor"]["subject"],
            "original-human"
        );
    }
    assert!(append(&WrongReceipt(MemoryAuditLedger::new()), &value).is_err());
}

#[test]
fn all_three_unknown_effect_shapes_keep_the_known_target_fact() {
    for phase in 0..3 {
        let mut value = rename_committed();
        let FileOperationKind::Rename { to, .. } = &value.kind else {
            unreachable!()
        };
        if phase == 0 {
            value.effects[0] = FileEffect::Unknown { path: to.clone() };
        }
        value.effects[1] = if phase == 2 {
            FileEffect::Unknown {
                path: value.path.clone(),
            }
        } else {
            FileEffect::NotApplied {
                path: value.path.clone(),
            }
        };
        value.outcome = Some(FileOperationOutcome::Indeterminate);
        value.tombstone_id = None;
        value.evidence_id = Some("7".repeat(64));
        let wrapped = event(&value).unwrap();
        assert_eq!(
            wrapped.payload().as_value()["file_operation"]["effects"],
            serde_json::to_value(&value.effects).unwrap()
        );
    }
    for outcome in [
        FileOperationOutcome::Rejected,
        FileOperationOutcome::Cancelled,
    ] {
        let mut value = rename_committed();
        let FileOperationKind::Rename { to, .. } = &value.kind else {
            unreachable!()
        };
        value.outcome = Some(outcome);
        value.tombstone_id = None;
        value.effects = vec![
            FileEffect::NotApplied { path: to.clone() },
            FileEffect::NotApplied {
                path: value.path.clone(),
            },
        ];
        event(&value).unwrap();
    }
}
