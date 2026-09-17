use super::*;

#[test]
fn invalid_phase_version_and_effects_never_reach_the_ledger() {
    let mut invalid = Vec::new();
    let mut value = intent(false);
    value.schema_version = 2;
    invalid.push(value);
    let mut value = intent(false);
    value.phase = ContentAuditPhase::Outcome;
    invalid.push(value);
    let mut value = intent(false);
    value.outcome = Some(FileOperationOutcome::Cancelled);
    invalid.push(value);
    let mut value = intent(false);
    value.evidence_id = Some("7".repeat(64));
    invalid.push(value);
    let mut value = intent(false);
    value.effects = committed(false).effects;
    invalid.push(value);
    let mut value = committed(false);
    value.effects.clear();
    invalid.push(value);
    let mut value = committed(false);
    value.effects.push(value.effects[0].clone());
    invalid.push(value);
    let mut value = committed(false);
    value.tombstone_id = None;
    invalid.push(value);
    let mut value = committed(false);
    value.tombstone_id = Some("8".repeat(64));
    invalid.push(value);
    let mut value = committed(true);
    value.tombstone_id = Some("8".repeat(64));
    invalid.push(value);
    let mut value = committed(false);
    value.evidence_id = Some("8".repeat(64));
    invalid.push(value);
    let mut value = committed(false);
    value.outcome = Some(FileOperationOutcome::Indeterminate);
    invalid.push(value);
    let mut value = committed(false);
    value.outcome = Some(FileOperationOutcome::Rejected);
    invalid.push(value);
    let mut value = committed(false);
    if let FileEffect::Confirmed { path, .. } = &mut value.effects[0] {
        *path = VaultPath::parse("other.md").unwrap();
    }
    invalid.push(value);
    let mut value = committed(false);
    if let FileEffect::Confirmed { before, after, .. } = &mut value.effects[0] {
        std::mem::swap(before, after);
    }
    invalid.push(value);
    let mut value = committed(false);
    if let FileOperationKind::Delete { expected } = &mut value.kind {
        *expected = Revision::compute(value.vault_uuid, &value.path, b"different");
    }
    invalid.push(value);
    let mut value = committed(true);
    if let FileEffect::Confirmed {
        after: FilePresence::Present { bytes, .. },
        ..
    } = &mut value.effects[0]
    {
        *bytes = 268_435_457;
    }
    invalid.push(value);
    for value in invalid {
        let ledger = MemoryAuditLedger::new();
        assert!(append(&ledger, &value).is_err(), "{value:?}");
        assert!(ledger.query(&AuditFilter::default()).unwrap().is_empty());
    }
}

#[test]
fn malformed_metadata_and_indeterminate_without_evidence_are_rejected() {
    let mut invalid = Vec::new();
    let mut value = intent(false);
    value.event_id = "A".repeat(64);
    invalid.push(value);
    let mut value = intent(false);
    value.operation_id.clear();
    invalid.push(value);
    let mut value = intent(false);
    value.request_fingerprint = "5".repeat(63);
    invalid.push(value);
    let mut value = intent(false);
    value.scope_epoch = 0;
    invalid.push(value);
    let mut value = intent(false);
    value.requested_at_ms = Some(-1);
    invalid.push(value);
    let mut value = intent(true);
    value.kind = FileOperationKind::RestoreTombstone {
        tombstone_id: "bad".into(),
    };
    invalid.push(value);
    let mut value = intent(false);
    value.actor = MutationActor::Human {
        issuer: "http://invalid".into(),
        subject: "subject".into(),
    };
    invalid.push(value);
    let mut value = intent(false);
    value.actor = MutationActor::Human {
        issuer: "https://valid.test".into(),
        subject: "\n".into(),
    };
    invalid.push(value);
    let mut value = intent(false);
    value.phase = ContentAuditPhase::Outcome;
    value.outcome = Some(FileOperationOutcome::Indeterminate);
    value.effects = vec![FileEffect::Unknown {
        path: value.path.clone(),
    }];
    invalid.push(value);
    for value in invalid {
        assert!(event(&value).is_err());
    }
}
