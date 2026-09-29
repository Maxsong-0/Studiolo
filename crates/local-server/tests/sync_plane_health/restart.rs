//! 轮末不能覆盖已确认的退出；持久冲突数在重开、回收和失败轮都保持准确。
use super::*;
use crate::sync_plane::recovery::PlaneStateStore;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn classified_exits_survive_round_failure_but_transport_failures_do_not_reuse_them() {
    let cases = [
        run(ExitObservation::Code(1), None),
        run(ExitObservation::Signal(9), None),
        run(ExitObservation::Code(0), Some(StopReason::DeadlineExceeded)),
        run(ExitObservation::Code(0), Some(StopReason::OwnerRequested)),
    ];
    for closed in cases {
        let health = after_clean_batch();
        health.record_headless(&closed, false);
        let classified = health.bridge().last_exit;
        health.record_round(11_000, &Err(closed.failure()));
        assert_eq!(health.bridge().last_exit, classified);
        assert_ne!(classified, Some(ExitClass::Clean));
        health.record_round(11_300, &Err(PlaneError::Transport));
        assert_eq!(health.bridge().last_exit, Some(ExitClass::Unconfirmed));
    }
}

#[test]
fn persisted_conflicts_survive_reopen_and_disappear_only_after_pruning() {
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let mut store = PlaneStateStore::open(root.path()).unwrap();
    let mut state = store.state().clone();
    state
        .conflict_stamps
        .insert("ab".repeat(32), "202609280001".to_owned());
    store.save(state).unwrap();
    drop(store);
    let mut reopened = PlaneStateStore::open(root.path()).unwrap();
    let health = PlaneHealth::new(None);
    health.record_conflicts(reopened.state().conflict_stamps.len() as u64);
    health.record_observed_copies(2);
    assert_eq!(health.bridge().conflict_copies, Some(3));
    // 同一持久集合重复回填不累加；副本尚未写完导致失败也不能回零。
    health.record_conflicts(reopened.state().conflict_stamps.len() as u64);
    health.record_round(11_000, &Err(PlaneError::Unavailable));
    assert_eq!(health.bridge().conflict_copies, Some(3));
    let mut state = reopened.state().clone();
    state.conflict_stamps.clear();
    reopened.save(state).unwrap();
    health.record_conflicts(reopened.state().conflict_stamps.len() as u64);
    assert_eq!(health.bridge().conflict_copies, Some(2));
    health.record_observed_copies(0);
    assert_eq!(health.bridge().conflict_copies, Some(0));
}
