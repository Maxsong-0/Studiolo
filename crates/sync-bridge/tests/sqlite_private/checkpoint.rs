//! 只验证数据库 owner 与租约机制；模拟 Gate 不充当真实传输静止或整链验收。
use super::support::*;
use crate::{Binding, Presence, sqlite::*};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use studiolo_audit_ledger::SqliteAuditLedger;
struct TestGate(AtomicBool);
impl BridgeCheckpointGate for TestGate {
    fn verify(&self, expected: &Binding, deadline: Instant) -> Result<(), BridgeError> {
        if expected == &binding() && Instant::now() < deadline && self.0.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(BridgeError::Unavailable)
        }
    }
}
fn gate() -> Arc<TestGate> {
    Arc::new(TestGate(AtomicBool::new(true)))
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
#[test]
fn checkpoint_source_retains_the_real_lease_and_exports_verified_independent_database() {
    let f = Fixture::new();
    let audit_dir = Fixture::new();
    let target = Fixture::new();
    let store = f.init();
    let source = store.checkpoint_source().unwrap();
    drop(store);
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::Busy)
    ));
    let ledger = SqliteAuditLedger::open(&audit_dir.root).unwrap();
    let audit = ledger
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    let mut frozen = source.freeze(gate(), deadline()).unwrap();
    assert!(frozen.export(&target.root).is_err());
    assert_eq!(frozen.verify_audit(&audit).unwrap().checked(), 0);
    frozen.export(&target.root).unwrap();
    let copy = rusqlite::Connection::open(target.root.join("bridge.sqlite3")).unwrap();
    super::super::database::sealed_schema(&copy).unwrap();
    let state = super::super::database::load(&copy).unwrap();
    assert_eq!(state.binding, binding());
    assert_eq!(state.version, 0);
    drop(source);
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::Busy)
    ));
    drop(frozen);
    assert_eq!(f.reopen().view().unwrap().version, 0);
}
#[test]
fn pending_effects_are_not_drained_and_every_retained_ack_is_checked() {
    let f = Fixture::new();
    let audit_dir = Fixture::new();
    let wrong_dir = Fixture::new();
    let target = Fixture::new();
    let mut store = f.init();
    let source = store.checkpoint_source().unwrap();
    let ledger = SqliteAuditLedger::open(&audit_dir.root).unwrap();
    let original = input(&store, 1, Presence::Missing, present(9));
    let action = reserve(&mut store, original.clone()).remove(0).action;
    assert!(source.freeze(gate(), deadline()).is_err());
    flush(&mut store, &ledger);
    let issued = store
        .issue(&action.key, check(&store, &original, &action))
        .unwrap();
    assert!(source.freeze(gate(), deadline()).is_err());
    store
        .record_outcome(&action.key, committed(&issued))
        .unwrap();
    flush(&mut store, &ledger);
    let observed = common(&store, &original, present(9));
    store
        .settle(store.view().unwrap().version, 1, Some(observed))
        .unwrap();
    let mut frozen = source.freeze(gate(), deadline()).unwrap();
    let wrong = SqliteAuditLedger::open(&wrong_dir.root)
        .unwrap()
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    assert!(frozen.verify_audit(&wrong).is_err());
    let audit = ledger
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    assert_eq!(frozen.verify_audit(&audit).unwrap().checked(), 3);
    frozen.export(&target.root).unwrap();
    let studiolo_core::checkpoint::OwnerWatermark::Bridge {
        observation_sequence,
        durable_gate_closed,
        ..
    } = frozen.watermark()
    else {
        panic!("wrong owner")
    };
    assert!(*observation_sequence > 0);
    assert!(*durable_gate_closed);
}
#[test]
fn expired_or_revoked_transport_gate_never_produces_a_sealed_receipt() {
    let f = Fixture::new();
    let audit_dir = Fixture::new();
    let target = Fixture::new();
    let store = f.init();
    let source = store.checkpoint_source().unwrap();
    let gate = gate();
    assert!(source.freeze(gate.clone(), Instant::now()).is_err());
    let mut frozen = source.freeze(gate.clone(), deadline()).unwrap();
    let audit = SqliteAuditLedger::open(&audit_dir.root)
        .unwrap()
        .checkpoint_source()
        .unwrap()
        .freeze(deadline())
        .unwrap();
    frozen.verify_audit(&audit).unwrap();
    gate.0.store(false, Ordering::SeqCst);
    assert!(frozen.export(&target.root).is_err());
    assert!(!target.root.join("bridge.sqlite3").exists());
}
