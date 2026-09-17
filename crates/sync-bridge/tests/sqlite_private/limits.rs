use super::support::*;
use crate::sqlite::*;
use crate::*;
use studiolo_audit_ledger::MemoryAuditLedger;

#[test]
fn fuse_and_cursor_commit_together_and_cannot_be_bypassed_after_reopen() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let base = input(&store, 60, present(9), present(9));
    reserve(&mut store, base.clone());
    flush(&mut store, &ledger);
    store
        .settle(
            store.view().unwrap().version,
            1,
            Some(common(&store, &base, present(9))),
        )
        .unwrap();
    let mut deletes = input(&store, 60, present(9), present(9));
    for value in deletes.canonical.entries.values_mut().take(6) {
        *value = Presence::Missing;
    }
    assert_eq!(
        store.reserve(store.view().unwrap().version, deletes),
        Ok(Reservation::Fused { sequence: 2 })
    );
    assert!(matches!(
        store.flush_audit(&RejectAudit, 1),
        Err(BridgeError::AuditUnavailable)
    ));
    drop(store);
    let mut store = f.reopen();
    assert!(store.view().unwrap().fused);
    assert_eq!(store.view().unwrap().cursor.last_plan_sequence, 2);
    let divided = input(&store, 1, Presence::Missing, present(9));
    assert_eq!(
        store.reserve(store.view().unwrap().version, divided),
        Err(BridgeError::Fused)
    );
    let mut rebound = binding();
    rebound.scope_epoch += 1;
    drop(store);
    assert!(matches!(
        BridgeState::open_existing(&f.root, rebound),
        Err(BridgeError::BindingMismatch)
    ));
    assert!(f.reopen().view().unwrap().fused);
}
#[test]
fn expired_check_blocks_issue_without_erasing_pending_or_changing_key() {
    let f = Fixture::new();
    let mut store = f.init();
    let ledger = MemoryAuditLedger::new();
    let original = input(&store, 1, Presence::Missing, present(9));
    let view = reserve(&mut store, original.clone()).remove(0);
    flush(&mut store, &ledger);
    let mut expired = check(&store, &original, &view.action);
    expired.expires_at = std::time::Instant::now();
    assert_eq!(
        store.issue(&view.action.key, expired),
        Err(BridgeError::StaleObservation)
    );
    assert_eq!(
        store.lookup(&view.action.key).unwrap().external_key,
        view.external_key
    );
    assert_eq!(store.view().unwrap().pending_sequence, Some(1));
}
#[test]
fn wrong_binding_and_i64_overflow_do_not_reserve_or_advance_cursor() {
    let f = Fixture::new();
    let mut store = f.init();
    let mut wrong = input(&store, 1, Presence::Missing, present(9));
    wrong.scope.binding.remote_vault = [99; 16];
    assert_eq!(store.reserve(0, wrong), Err(BridgeError::BindingMismatch));
    let mut overflow = input(&store, 1, Presence::Missing, present(9));
    overflow.plan_sequence = i64::MAX as u64 + 1;
    assert_eq!(store.reserve(0, overflow), Err(BridgeError::Overflow));
    assert_eq!(store.view().unwrap().version, 0);
    assert_eq!(store.view().unwrap().cursor.last_plan_sequence, 0);
}
#[test]
fn decoder_rejects_huge_count_before_allocating_collection() {
    let f = Fixture::new();
    let store = f.init();
    let original = input(&store, 1, present(9), present(9));
    let mut bytes = super::super::input_codec::encode(&original).unwrap();
    let count_offset = 8 + 16 + 16 + 16 + 32 + 32 + 32 + 8;
    bytes[count_offset..count_offset + 8].copy_from_slice(&4097u64.to_be_bytes());
    assert_eq!(
        super::super::input_codec::decode(&bytes),
        Err(BridgeError::Budget)
    );
    let huge = vec![0; super::super::INPUT_MAX + 1];
    assert_eq!(
        super::super::input_codec::decode(&huge),
        Err(BridgeError::Budget)
    );
}
#[test]
fn invalid_storage_budget_never_initializes() {
    let f = Fixture::new();
    let mut limits = Limits::conservative(1_000_000);
    limits.max_paths = 4097;
    assert!(matches!(
        BridgeState::initialize(&f.root, binding(), limits),
        Err(BridgeError::Budget)
    ));
    assert_eq!(std::fs::read_dir(&f.root).unwrap().count(), 0);
}
#[test]
fn long_reader_prevents_further_writes_until_old_wal_can_be_truncated() {
    let f = Fixture::new();
    let mut store = f.init();
    let external = rusqlite::Connection::open(f.root.join("bridge.sqlite3")).unwrap();
    external.execute_batch("BEGIN;").unwrap();
    let _: i64 = external
        .query_row("SELECT revision FROM bridge_meta", [], |r| r.get(0))
        .unwrap();
    let candidate = input(&store, 1, Presence::Missing, present(9));
    // 可能在写前因旧 reader 拒绝，也可能完成首次 commit 后的 checkpoint 拒绝。
    assert!(store.reserve(0, candidate).is_err());
    assert!(store.view().is_err() || store.view().unwrap().version == 0);
    drop(store);
    if std::fs::metadata(f.root.join("bridge.sqlite3-wal")).is_ok_and(|m| m.len() > 0) {
        assert!(BridgeState::open_existing(&f.root, binding()).is_err());
    }
    external.execute_batch("ROLLBACK;").unwrap();
    drop(external);
    let store = f.reopen();
    assert!(store.view().unwrap().version <= 1);
}
