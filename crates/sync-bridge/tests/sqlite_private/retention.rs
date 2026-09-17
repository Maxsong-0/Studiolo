use super::support::*;
use crate::sqlite::*;
use crate::*;
use studiolo_audit_ledger::MemoryAuditLedger;

#[test]
fn retirement_needs_explicit_ticket_and_real_ack_then_old_keys_are_retired() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let first = input(&store, 1, Presence::Missing, present(9));
    let a = reserve(&mut store, first.clone()).remove(0).action;
    flush(&mut store, &ledger);
    let issued = store.issue(&a.key, check(&store, &first, &a)).unwrap();
    store.record_outcome(&a.key, committed(&issued)).unwrap();
    flush(&mut store, &ledger);
    store
        .settle(
            store.view().unwrap().version,
            1,
            Some(common(&store, &first, present(9))),
        )
        .unwrap();
    let second = input(&store, 1, present(9), present(10));
    let b = reserve(&mut store, second.clone()).remove(0).action;
    flush(&mut store, &ledger);
    let issued_b = store.issue(&b.key, check(&store, &second, &b)).unwrap();
    store.record_outcome(&b.key, committed(&issued_b)).unwrap();
    flush(&mut store, &ledger);
    let candidate = common(&store, &second, present(10));
    assert_eq!(
        store.settle(store.view().unwrap().version, 2, Some(candidate.clone())),
        Err(BridgeError::RetentionRequired)
    );
    store
        .prepare_retirement(store.view().unwrap().version)
        .unwrap();
    assert!(matches!(
        store.flush_audit(&RejectAudit, 1),
        Err(BridgeError::AuditUnavailable)
    ));
    assert_eq!(
        store.settle(store.view().unwrap().version, 2, Some(candidate)),
        Err(BridgeError::RetentionRequired)
    );
    assert!(matches!(
        store.lookup(&a.key).unwrap().status,
        ActionStatus::Known(_)
    ));
    drop(store);
    let mut store = f.reopen();
    flush(&mut store, &ledger);
    let fresh = common(&store, &second, present(10));
    store
        .settle(store.view().unwrap().version, 2, Some(fresh))
        .unwrap();
    assert_eq!(store.lookup(&a.key), Err(BridgeError::Retired));
    assert_eq!(
        store.record_outcome(&a.key, committed(&issued)),
        Err(BridgeError::Retired)
    );
    assert!(matches!(
        store.lookup(&b.key).unwrap().status,
        ActionStatus::Known(_)
    ));
    drop(store);
    let store = f.reopen();
    assert_eq!(store.view().unwrap().retired_through, 1);
}
#[test]
fn partial_failure_preserves_common_baseline_and_both_original_outcomes() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let initial = input(&store, 2, present(9), present(9));
    reserve(&mut store, initial.clone());
    flush(&mut store, &ledger);
    store
        .settle(
            store.view().unwrap().version,
            1,
            Some(common(&store, &initial, present(9))),
        )
        .unwrap();
    let old_base = store.view().unwrap().baseline;
    let update = input(&store, 2, present(9), present(10));
    let actions = reserve(&mut store, update.clone());
    flush(&mut store, &ledger);
    for (i, a) in actions.iter().enumerate() {
        let issued = store
            .issue(&a.action.key, check(&store, &update, &a.action))
            .unwrap();
        store
            .record_outcome(
                &a.action.key,
                if i == 0 {
                    committed(&issued)
                } else {
                    rejected(&issued)
                },
            )
            .unwrap();
    }
    flush(&mut store, &ledger);
    store
        .prepare_retirement(store.view().unwrap().version)
        .unwrap();
    flush(&mut store, &ledger);
    assert_eq!(
        store.settle(
            store.view().unwrap().version,
            2,
            Some(common(&store, &update, present(10)))
        ),
        Err(BridgeError::NotSettled)
    );
    store
        .settle(store.view().unwrap().version, 2, None)
        .unwrap();
    assert_eq!(store.view().unwrap().baseline, old_base);
    drop(store);
    let store = f.reopen();
    for a in actions {
        assert!(matches!(
            store.lookup(&a.action.key).unwrap().status,
            ActionStatus::Known(_)
        ));
    }
}
#[test]
fn a_to_b_to_a_uses_new_plan_keys() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let mut keys = Vec::new();
    for (old, new) in [
        (Presence::Missing, present(9)),
        (present(9), present(10)),
        (present(10), present(9)),
    ] {
        let original = input(&store, 1, old, new.clone());
        let seq = original.plan_sequence;
        let a = reserve(&mut store, original.clone()).remove(0).action;
        flush(&mut store, &ledger);
        let issued = store.issue(&a.key, check(&store, &original, &a)).unwrap();
        keys.push(issued.external_key.clone());
        store.record_outcome(&a.key, committed(&issued)).unwrap();
        flush(&mut store, &ledger);
        if seq > 1 {
            store
                .prepare_retirement(store.view().unwrap().version)
                .unwrap();
            flush(&mut store, &ledger);
        }
        store
            .settle(
                store.view().unwrap().version,
                seq,
                Some(common(&store, &original, new)),
            )
            .unwrap();
    }
    assert_ne!(keys[0], keys[2]);
    assert_eq!(store.view().unwrap().cursor.last_plan_sequence, 3);
}
