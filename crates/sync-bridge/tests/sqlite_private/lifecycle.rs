use super::support::*;
use crate::sqlite::*;
use crate::*;
use std::sync::atomic::{AtomicBool, Ordering};
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditRecord, MemoryAuditLedger,
    SqliteAuditLedger,
};

#[test]
fn intent_ack_is_required_and_issued_reopens_unknown_with_original_key() {
    let f = Fixture::new();
    let mut store = f.init();
    let input = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, input.clone()).remove(0).action;
    let old_check = check(&store, &input, &action);
    assert_eq!(
        store.issue(&action.key, old_check.clone()),
        Err(BridgeError::IntentRequired)
    );
    assert!(matches!(
        store.flush_audit(&RejectAudit, 1),
        Err(BridgeError::AuditUnavailable)
    ));
    let ledger = MemoryAuditLedger::new();
    flush(&mut store, &ledger);
    let issued = store.issue(&action.key, old_check.clone()).unwrap();
    drop(store);
    let mut store = f.reopen();
    let progress = store.lookup(&action.key).unwrap();
    assert_eq!(progress.external_key, issued.external_key);
    assert_eq!(progress.status, ActionStatus::OutcomeUnknown);
    assert_eq!(
        store.issue(&action.key, old_check),
        Err(BridgeError::StaleObservation)
    );
    assert!(matches!(
        store.reserve(store.view().unwrap().version, input.clone()),
        Err(BridgeError::PendingUnresolved)
    ));
    store
        .record_outcome(&action.key, committed(&issued))
        .unwrap();
    flush(&mut store, &ledger);
    let common = common(&store, &input, present(9));
    store
        .settle(store.view().unwrap().version, 1, Some(common))
        .unwrap();
    assert_eq!(store.view().unwrap().baseline.version, 1);
    drop(store);
    assert_eq!(
        f.reopen().lookup(&action.key).unwrap().status,
        ActionStatus::Known(Box::new(committed(&issued)))
    );
}
#[test]
fn same_content_does_not_resolve_unknown_and_frozen_input_cannot_change() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    flush(&mut store, &ledger);
    store
        .issue(&action.key, check(&store, &original, &action))
        .unwrap();
    let mut changed = original.clone();
    changed.canonical.entries = changed.mirror.entries.clone();
    assert_eq!(
        store.retry_pending(&changed),
        Err(BridgeError::RetryMismatch)
    );
    let equal = common(&store, &original, present(9));
    assert_eq!(
        store.settle(store.view().unwrap().version, 1, Some(equal)),
        Err(BridgeError::NotSettled)
    );
    assert_eq!(store.view().unwrap().baseline.version, 0);
}
#[test]
fn outcome_wrong_key_backend_revision_and_request_digest_are_rejected() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    flush(&mut store, &ledger);
    let issued = store
        .issue(&action.key, check(&store, &original, &action))
        .unwrap();
    let good = committed(&issued);
    for variant in 0..4 {
        let mut bad = good.clone();
        match variant {
            0 => bad.external_key.push('x'),
            1 => {
                bad.backend = ReceiptBackend::Mirror {
                    mirror_root: [5; 32],
                }
            }
            2 => bad.request_digest = [0; 32],
            _ => {
                if let ReceiptOutcome::Committed {
                    previous_revision, ..
                } = &mut bad.outcome
                {
                    *previous_revision = Some([8; 32]);
                }
            }
        }
        assert_eq!(
            store.record_outcome(&action.key, bad),
            Err(BridgeError::OutcomeMismatch)
        );
    }
    store.record_outcome(&action.key, good.clone()).unwrap();
    store.record_outcome(&action.key, good).unwrap();
    assert_eq!(
        store.record_outcome(&action.key, rejected(&issued)),
        Err(BridgeError::OutcomeMismatch)
    );
}
#[test]
fn zero_actions_require_summary_and_new_complete_observation_to_advance() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let original = input(&store, 1, present(9), present(9));
    assert!(reserve(&mut store, original.clone()).is_empty());
    let good = common(&store, &original, present(9));
    assert_eq!(
        store.settle(store.view().unwrap().version, 1, Some(good.clone())),
        Err(BridgeError::NotSettled)
    );
    flush(&mut store, &ledger);
    let mut partial = good.clone();
    partial.mirror.complete = false;
    assert_eq!(
        store.settle(store.view().unwrap().version, 1, Some(partial)),
        Err(BridgeError::StaleObservation)
    );
    drop(store);
    let mut store = f.reopen();
    assert_eq!(
        store.settle(store.view().unwrap().version, 1, Some(good)),
        Err(BridgeError::StaleObservation)
    );
    let fresh = common(&store, &original, present(9));
    store
        .settle(store.view().unwrap().version, 1, Some(fresh))
        .unwrap();
    assert_eq!(store.view().unwrap().baseline.version, 1);
}

struct LoseFirstReply {
    ledger: SqliteAuditLedger,
    lose: AtomicBool,
}
impl AuditLedger for LoseFirstReply {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError> {
        let receipt = self.ledger.append(event)?;
        if self.lose.swap(false, Ordering::SeqCst) {
            return Err(AuditError::Unavailable);
        }
        Ok(receipt)
    }
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
        self.ledger.query(filter)
    }
}
#[test]
fn actual_sqlite_audit_append_then_lost_reply_replays_exact_intent_after_restart() {
    let f = Fixture::new();
    let a = Fixture::new();
    let mut store = f.init();
    let ledger = LoseFirstReply {
        ledger: SqliteAuditLedger::open(&a.root).unwrap(),
        lose: AtomicBool::new(true),
    };
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    assert!(matches!(
        store.flush_audit(&ledger, 1),
        Err(BridgeError::AuditUnavailable)
    ));
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 1);
    drop(store);
    let mut store = f.reopen();
    flush(&mut store, &ledger);
    assert_eq!(ledger.query(&AuditFilter::default()).unwrap().len(), 1);
    assert_eq!(
        store.lookup(&action.key).unwrap().status,
        ActionStatus::Reserved
    );
}
