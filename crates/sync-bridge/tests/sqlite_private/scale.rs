use super::support::*;
use crate::sqlite::*;
use crate::*;
use studiolo_audit_ledger::MemoryAuditLedger;

#[test]
fn near_budget_terminal_outcomes_use_individual_rows_without_rewriting_input() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let original = input(&store, 1800, Presence::Missing, present(9));
    let actions = reserve(&mut store, original.clone());
    assert_eq!(actions.len(), 1800);
    let reserved = super::super::capacity::reserved_bytes(&store.cache).unwrap();
    assert!(reserved > super::super::STATE_MAX * 9 / 10 && reserved <= super::super::STATE_MAX);
    let before = store.statistics();
    assert_eq!(before.full_loads, 1);
    assert_eq!(before.full_writes, 1);
    let input_bytes = {
        let read = rusqlite::Connection::open(f.root.join("bridge.sqlite3")).unwrap();
        read.query_row(
            "SELECT input FROM bridge_plans WHERE role='pending'",
            [],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .unwrap()
    };
    flush(&mut store, &ledger);
    for view in &actions {
        let issued = store
            .issue(&view.action.key, check(&store, &original, &view.action))
            .unwrap();
        let mut receipt = committed(&issued);
        if let ReceiptOutcome::Committed { receipt_id, .. } = &mut receipt.outcome {
            *receipt_id = "x".repeat(256);
        }
        store.record_outcome(&view.action.key, receipt).unwrap();
    }
    flush(&mut store, &ledger);
    let after = store.statistics();
    assert_eq!(after.full_loads, before.full_loads);
    assert_eq!(after.full_writes, before.full_writes);
    assert_eq!(after.action_writes, 7200);
    let read = rusqlite::Connection::open(f.root.join("bridge.sqlite3")).unwrap();
    let same: Vec<u8> = read
        .query_row(
            "SELECT input FROM bridge_plans WHERE role='pending'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(same, input_bytes);
    let rows: i64 = read
        .query_row("SELECT count(*) FROM bridge_actions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 1800);
    drop(read);
    assert_eq!(
        std::fs::metadata(f.root.join("bridge.sqlite3-wal"))
            .unwrap()
            .len(),
        0
    );
    drop(store);
    let mut store = f.reopen();
    store
        .settle(
            store.view().unwrap().version,
            1,
            Some(common(&store, &original, present(9))),
        )
        .unwrap();
    assert_eq!(store.view().unwrap().baseline.entries.len(), 1800);
    for view in actions {
        assert!(matches!(
            store.lookup(&view.action.key).unwrap().status,
            ActionStatus::Known(_)
        ));
    }
}

#[test]
fn over_budget_future_receipts_reject_entire_plan_before_any_intent_or_issue() {
    let f = Fixture::new();
    let mut store = f.init();
    let candidate = input(&store, 2048, Presence::Missing, present(9));
    assert_eq!(store.reserve(0, candidate), Err(BridgeError::Budget));
    assert_eq!(store.view().unwrap().version, 0);
    assert_eq!(store.view().unwrap().cursor.last_plan_sequence, 0);
    assert!(store.frozen_pending().unwrap().is_none());
    assert!(store.pending_audit().unwrap().is_none());
    drop(store);
    assert!(f.reopen().frozen_pending().unwrap().is_none());
}
