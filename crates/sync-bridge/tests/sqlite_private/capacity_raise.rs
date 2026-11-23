//! 持久预算只升容量（BUG-047）：旧链 `sync-init` 时冻结的 4096 升到同步路径预算后，
//! 冻结在旧预算的 pending／last／fuse 照旧有效、续做与结算照旧，新预留只认新预算。
use super::support::*;
use crate::sqlite::*;
use crate::*;
use studiolo_audit_ledger::MemoryAuditLedger;

/// 169 上的旧链：`sync-init` 时的保守预算还是 4096。
fn old_limits() -> Limits {
    Limits {
        max_paths: 4096,
        max_outputs: 4096,
        ..Limits::conservative(1_000_000)
    }
}

fn new_limits() -> Limits {
    Limits {
        max_paths: studiolo_core::SYNC_PATH_BUDGET,
        max_outputs: studiolo_core::SYNC_PATH_BUDGET,
        ..old_limits()
    }
}

fn old_chain(f: &Fixture) -> BridgeState {
    BridgeState::initialize(&f.root, binding(), old_limits()).unwrap()
}

/// 冻结在旧预算的一批：预留、Intent ACK、提交回执、结果 ACK，都不结算。
fn committed_batch(
    store: &mut BridgeState,
    ledger: &MemoryAuditLedger,
    old: Presence,
    new: Presence,
    limits: Limits,
) -> Input {
    let mut original = input(store, 1, old, new);
    original.limits = limits;
    let action = reserve(store, original.clone()).remove(0).action;
    flush(store, ledger);
    let issued = store
        .issue(&action.key, check(store, &original, &action))
        .unwrap();
    store
        .record_outcome(&action.key, committed(&issued))
        .unwrap();
    flush(store, ledger);
    original
}

#[test]
fn raised_capacity_keeps_frozen_last_and_pending_valid_and_new_reserves_use_it() {
    let f = Fixture::new();
    let mut store = old_chain(&f);
    let ledger = MemoryAuditLedger::new();
    let first = committed_batch(
        &mut store,
        &ledger,
        Presence::Missing,
        present(9),
        old_limits(),
    );
    store
        .settle(
            store.view().unwrap().version,
            1,
            Some(common(&store, &first, present(9))),
        )
        .unwrap();
    // 第 2 批冻结在旧预算、还没结算：升级时 last（第 1 批）与 pending（第 2 批）都在。
    let second = committed_batch(&mut store, &ledger, present(9), present(10), old_limits());
    let before = store.view().unwrap().version;
    assert_eq!(store.raise_capacity(new_limits()), Ok(Some(4096)));
    assert_eq!(store.view().unwrap().version, before + 1);
    let raised = store.view().unwrap().version;
    assert_eq!(
        store.raise_capacity(new_limits()),
        Ok(None),
        "已是目标值不写库"
    );
    assert_eq!(store.view().unwrap().version, raised);
    drop(store);

    let mut store = f.reopen();
    let view = store.view().unwrap();
    assert_eq!(
        (view.pending_sequence, view.last_sequence),
        (Some(2), Some(1))
    );
    store
        .prepare_retirement(store.view().unwrap().version)
        .unwrap();
    flush(&mut store, &ledger);
    store
        .settle(
            store.view().unwrap().version,
            2,
            Some(common(&store, &second, present(10))),
        )
        .unwrap();
    assert_eq!(store.view().unwrap().pending_sequence, None);

    let mut stale = input(&store, 1, present(10), present(9));
    stale.limits = old_limits();
    assert_eq!(
        store.reserve(store.view().unwrap().version, stale),
        Err(BridgeError::VersionConflict),
        "升级后旧预算的新输入不再被接受"
    );
    let mut third = input(&store, 1, present(10), present(9));
    third.limits = new_limits();
    assert_eq!(
        store.reserve(store.view().unwrap().version, third),
        Ok(Reservation::Ready { sequence: 3 })
    );
    drop(store);
    assert_eq!(f.reopen().view().unwrap().pending_sequence, Some(3));
}

#[test]
fn raised_capacity_keeps_a_fuse_frozen_in_the_old_budget() {
    let f = Fixture::new();
    let mut store = old_chain(&f);
    let ledger = MemoryAuditLedger::new();
    let mut base = input(&store, 60, present(9), present(9));
    base.limits = old_limits();
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
    deletes.limits = old_limits();
    for value in deletes.canonical.entries.values_mut().take(6) {
        *value = Presence::Missing;
    }
    assert_eq!(
        store.reserve(store.view().unwrap().version, deletes),
        Ok(Reservation::Fused { sequence: 2 })
    );
    assert_eq!(store.raise_capacity(new_limits()), Ok(Some(4096)));
    drop(store);
    let store = f.reopen();
    assert!(store.view().unwrap().fused, "升级不解除熔断");
}

#[test]
fn only_a_pure_capacity_raise_is_accepted() {
    let f = Fixture::new();
    let mut store = old_chain(&f);
    let version = store.view().unwrap().version;
    let mut smaller = old_limits();
    smaller.max_paths = 2048;
    let mut outputs_smaller = new_limits();
    outputs_smaller.max_outputs = 2048;
    let mut deletion = new_limits();
    deletion.deletion.max_count = 6;
    let mut bytes = new_limits();
    bytes.max_file_bytes += 1;
    let mut over = new_limits();
    over.max_paths = studiolo_core::SYNC_PATH_BUDGET + 1;
    for target in [smaller, outputs_smaller, deletion, bytes, over] {
        assert_eq!(
            store.raise_capacity(target),
            Err(BridgeError::Budget),
            "{target:?}"
        );
        assert_eq!(store.view().unwrap().version, version);
    }
    drop(store);
    // 拒绝之后库里仍是旧预算：同一条升级照样能做。
    let mut store = f.reopen();
    assert_eq!(store.raise_capacity(new_limits()), Ok(Some(4096)));
}
