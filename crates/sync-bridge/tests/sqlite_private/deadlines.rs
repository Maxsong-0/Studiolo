//! 可控事务 seam：实际截止点过去以后才继续，不以墙钟执行速度断言正确性。
use super::support::*;
use crate::sqlite::*;
use crate::*;
use std::time::{Duration, Instant};
use studiolo_audit_ledger::MemoryAuditLedger;

fn until(deadline: Instant) -> Box<dyn FnOnce()> {
    Box::new(move || {
        std::thread::sleep(
            deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
        )
    })
}
#[test]
fn expiry_while_waiting_for_transaction_does_not_issue() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let original = input(&store, 1, Presence::Missing, present(9));
    let a = reserve(&mut store, original.clone()).remove(0).action;
    flush(&mut store, &ledger);
    let mut proof = check(&store, &original, &a);
    proof.expires_at = Instant::now() + Duration::from_secs(1);
    store.before_guard = Some(until(proof.expires_at));
    assert_eq!(
        store.issue(&a.key, proof),
        Err(BridgeError::StaleObservation)
    );
    assert!(
        store.before_guard.is_none(),
        "must exercise post-lock expiry check"
    );
    assert_eq!(store.lookup(&a.key).unwrap().status, ActionStatus::Reserved);
}
#[test]
fn expiry_after_commit_keeps_issued_unknown_and_never_returns_dispatch() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let original = input(&store, 1, Presence::Missing, present(9));
    let a = reserve(&mut store, original.clone()).remove(0).action;
    flush(&mut store, &ledger);
    let mut proof = check(&store, &original, &a);
    proof.expires_at = Instant::now() + Duration::from_secs(1);
    store.after_commit = Some(until(proof.expires_at));
    assert_eq!(
        store.issue(&a.key, proof),
        Err(BridgeError::StaleObservation)
    );
    assert!(
        store.after_commit.is_none(),
        "must reach durable Issued before deadline expires"
    );
    assert_eq!(
        store.lookup(&a.key).unwrap().status,
        ActionStatus::OutcomeUnknown
    );
    drop(store);
    assert_eq!(
        f.reopen().lookup(&a.key).unwrap().status,
        ActionStatus::OutcomeUnknown
    );
}
#[test]
fn expiry_after_lock_does_not_advance_zero_action_common_baseline() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let original = input(&store, 1, present(9), present(9));
    reserve(&mut store, original.clone());
    flush(&mut store, &ledger);
    let mut proof = common(&store, &original, present(9));
    proof.expires_at = Instant::now() + Duration::from_secs(1);
    store.before_guard = Some(until(proof.expires_at));
    assert_eq!(
        store.settle(store.view().unwrap().version, 1, Some(proof)),
        Err(BridgeError::StaleObservation)
    );
    assert!(store.before_guard.is_none());
    assert_eq!(store.view().unwrap().baseline.version, 0);
    assert_eq!(store.view().unwrap().pending_sequence, Some(1));
}
