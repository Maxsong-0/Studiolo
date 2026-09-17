//! 确定性复制同一个持锁 FD；模拟继承生命周期，不声称复现特定 fork 调度。
use super::support::*;
use crate::sqlite::storage::Storage;
use crate::sqlite::*;
use std::fs::{self, TryLockError};
use std::os::unix::fs::PermissionsExt;

#[test]
fn owner_drop_unlocks_even_when_duplicated_lease_remains_alive() {
    let f = Fixture::new();
    let store = f.init();
    let inherited = store.storage.duplicate_lease_for_test().unwrap();
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::Busy)
    ));
    drop(store);
    // 不重试、不睡眠；副本仍然存在时必须立即允许下一 owner。
    let next = f.reopen();
    assert_eq!(next.view().unwrap().version, 0);
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::Busy)
    ));
    drop(inherited);
    // 旧副本的关闭不能解除新 owner 的另一 open file description 上的锁。
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::Busy)
    ));
    drop(next);
    assert_eq!(f.reopen().view().unwrap().version, 0);
}

#[test]
fn construction_failure_after_acquiring_lease_releases_inherited_copy_lock() {
    let f = Fixture::new();
    drop(f.init());
    let mut inherited = None;
    let result = Storage::open_inner(
        &f.root,
        false,
        Some(&mut |file| {
            inherited = Some(file.try_clone().map_err(|_| BridgeError::Unavailable)?);
            // 发生在取得租约之后、DB 普通文件核验之前的真实失败路径。
            fs::set_permissions(
                f.root.join("bridge.sqlite3"),
                fs::Permissions::from_mode(0o640),
            )
            .unwrap();
            Ok(())
        }),
    );
    assert!(matches!(result, Err(BridgeError::UnsafeStorage)));
    assert!(inherited.is_some());
    fs::set_permissions(
        f.root.join("bridge.sqlite3"),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let next = f.reopen();
    assert_eq!(next.view().unwrap().version, 0);
    drop(inherited);
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::Busy)
    ));
}

#[test]
fn connection_drop_rolls_back_before_the_next_owner_loads_with_old_lease_copy_alive() {
    let f = Fixture::new();
    let store = f.init();
    let inherited = store.storage.duplicate_lease_for_test().unwrap();
    store
        .connection
        .execute_batch("BEGIN IMMEDIATE; UPDATE bridge_meta SET revision=revision+1;")
        .unwrap();
    drop(store);
    let next = f.reopen();
    assert_eq!(next.view().unwrap().version, 0);
    drop(inherited);
    assert!(matches!(
        BridgeState::open_existing(&f.root, binding()),
        Err(BridgeError::Busy)
    ));
}

#[test]
fn lock_io_failure_is_not_reported_as_another_owner() {
    use crate::sqlite::storage::lock_error;
    assert_eq!(lock_error(TryLockError::WouldBlock), BridgeError::Busy);
    assert_eq!(
        lock_error(TryLockError::Error(std::io::Error::other(
            "synthetic lock I/O failure"
        ))),
        BridgeError::Unavailable
    );
}
