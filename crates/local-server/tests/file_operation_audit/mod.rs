//! 合成固定事件只检验装配契约；真实身份与文件执行另由 owner/Broker 验证。
mod malformed;

use super::*;
use studiolo_audit_ledger::{AuditFilter, AuditRecord, MemoryAuditLedger};
use studiolo_core::{Revision, VaultPath, VaultUuid};
use studiolo_vault_broker::FileOperationScopeHash;

fn intent(restore: bool) -> FileOperationAuditEvent {
    let path = VaultPath::parse("note.md").unwrap();
    let vault_uuid = VaultUuid::new();
    FileOperationAuditEvent {
        schema_version: 1,
        event_id: "1".repeat(64),
        operation_id: "2".repeat(64),
        phase: ContentAuditPhase::Intent,
        vault_uuid,
        actor: MutationActor::Human {
            issuer: "https://fixture.test".into(),
            subject: "original-human".into(),
        },
        kind: if restore {
            FileOperationKind::RestoreTombstone {
                tombstone_id: "3".repeat(64),
            }
        } else {
            FileOperationKind::Delete {
                expected: Revision::compute(vault_uuid, &path, b"old"),
            }
        },
        path,
        scope_hash: FileOperationScopeHash::new("4".repeat(64)).unwrap(),
        scope_epoch: 1,
        requested_at_ms: None,
        request_fingerprint: "5".repeat(64),
        outcome: None,
        effects: vec![],
        tombstone_id: None,
        evidence_id: None,
    }
}

fn committed(restore: bool) -> FileOperationAuditEvent {
    let mut value = intent(restore);
    value.phase = ContentAuditPhase::Outcome;
    value.outcome = Some(FileOperationOutcome::Committed);
    let present = FilePresence::Present {
        revision: Revision::compute(value.vault_uuid, &value.path, b"old"),
        sha256: "6".repeat(64),
        bytes: 3,
    };
    let (before, after) = if restore {
        (FilePresence::Missing, present)
    } else {
        (present, FilePresence::Missing)
    };
    value.effects = vec![FileEffect::Confirmed {
        path: value.path.clone(),
        before,
        after,
    }];
    value.tombstone_id = (!restore).then(|| value.operation_id.clone());
    value
}

#[test]
fn complete_broker_v1_is_wrapped_in_distinct_audit_v2_with_original_actor() {
    for restore in [false, true] {
        let ledger = MemoryAuditLedger::new();
        let value = intent(restore);
        let original = serde_json::to_value(&value).unwrap();
        let first = append(&ledger, &value).unwrap();
        assert_eq!(append(&ledger, &value).unwrap(), first);
        let records = ledger.query(&AuditFilter::default()).unwrap();
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.event().phase(), AuditPhase::Intent);
        assert_eq!(record.event().event_id().as_str(), value.event_id);
        assert_eq!(
            record.event().action(),
            if restore {
                "recovery.page_restore"
            } else {
                "vault.mutation"
            }
        );
        assert_eq!(
            record.event().payload().as_value(),
            &serde_json::json!({
                "schema_version":2,"file_operation":original
            })
        );
        assert_eq!(serde_json::to_value(&value).unwrap(), original);
        record.verify_integrity().unwrap();
    }
}

#[test]
fn outcome_closed_set_preserves_missing_and_zero_byte_present() {
    for restore in [false, true] {
        let mut value = committed(restore);
        if let FileEffect::Confirmed { before, after, .. } = &mut value.effects[0] {
            let present = if restore { after } else { before };
            if let FilePresence::Present { bytes, .. } = present {
                *bytes = 0;
            }
        }
        event(&value).unwrap();
    }
    for outcome in [
        FileOperationOutcome::Rejected,
        FileOperationOutcome::Cancelled,
        FileOperationOutcome::Indeterminate,
    ] {
        let mut value = intent(false);
        value.phase = ContentAuditPhase::Outcome;
        value.outcome = Some(outcome);
        value.effects = vec![if outcome == FileOperationOutcome::Indeterminate {
            value.evidence_id = Some("7".repeat(64));
            FileEffect::Unknown {
                path: value.path.clone(),
            }
        } else {
            FileEffect::NotApplied {
                path: value.path.clone(),
            }
        }];
        event(&value).unwrap();
    }
}

struct WrongReceipt(MemoryAuditLedger);
impl AuditLedger for WrongReceipt {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let mut payload = event.payload().as_value().clone();
        payload["file_operation"]["scope_epoch"] = 99.into();
        self.0.append(AuditEvent::new(
            event.event_id().clone(),
            event.phase(),
            event.action(),
            SanitizedPayload::try_from(payload).unwrap(),
        ))
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.0.query(filter)
    }
}

#[test]
fn valid_integrity_for_changed_payload_is_not_the_original_ack() {
    let ledger = WrongReceipt(MemoryAuditLedger::new());
    assert!(matches!(
        append(&ledger, &intent(false)),
        Err(AuditError::CorruptStore)
    ));
}
mod rename;
mod rename_malformed;
