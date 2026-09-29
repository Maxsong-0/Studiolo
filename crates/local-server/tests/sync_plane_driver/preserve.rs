//! 双向冲突保全（评审 P2-2）：Bidirectional 下有冲突先保全双方版本再照常执行，MirrorRemote 仍走预解决；
//! 保全失败本轮停在执行之前；冲突身份与首次保全戳跨轮稳定，同一冲突只配一次戳。

use super::NOW;
use super::fake_ports::{Call, FakePorts};
use super::fixture;
use std::collections::BTreeMap;
use studiolo_core::VaultPath;
use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::driver::preserve::{assign_stamps, conflict_key};
use studiolo_local_server::sync_plane::driver::{RoundOutcome, run_round_with};
use studiolo_sync_bridge::{Conflict, Decision, Presence, plan};
use studiolo_sync_transport::owned_run::SyncMode;

const LINK: [u8; 16] = [0x11; 16];

fn decision(conflicts: u64) -> Decision {
    plan(fixture::conflict_input(conflicts), None).expect("固定夹具必须可规划")
}

fn first_conflict(decision: &Decision) -> Conflict {
    match decision {
        Decision::Ready(plan) => plan.conflicts()[0].clone(),
        Decision::FuseBlocked(blocked) => blocked.conflicts[0].clone(),
    }
}

#[test]
fn bidirectional_conflicts_are_preserved_before_execute_and_never_preresolved() {
    let mode = SyncMode::Bidirectional;
    let mut ports = FakePorts::healthy(mode);
    ports.input = fixture::conflict_input(2);
    ports.preserve_result = Ok(2);
    let outcome = run_round_with(&mut ports, mode, NOW);
    assert!(
        matches!(outcome, Ok(RoundOutcome::Settled { .. })),
        "{outcome:?}"
    );
    assert_eq!(ports.count(&Call::Preresolve), 0);
    let tail = &ports.calls[ports.calls.len() - 3..];
    assert_eq!(tail, &[Call::Observe(mode), Call::Preserve, Call::Execute]);
    // 没有冲突就不保全。
    let mut ports = FakePorts::healthy(mode);
    let _ = run_round_with(&mut ports, mode, NOW);
    assert_eq!(ports.count(&Call::Preserve), 0);
    assert_eq!(ports.count(&Call::Execute), 1);
}

#[test]
fn mirror_remote_conflicts_still_go_through_preresolve_and_are_not_preserved() {
    let mode = SyncMode::MirrorRemote;
    let mut ports = FakePorts::healthy(mode);
    ports.input = fixture::conflict_input(2);
    let _ = run_round_with(&mut ports, mode, NOW);
    assert_eq!(ports.count(&Call::Preresolve), 1);
    assert_eq!(ports.count(&Call::Preserve), 0);
    assert_eq!(ports.count(&Call::Execute), 1);
}

#[test]
fn fuse_blocked_batches_with_conflicts_are_preserved_before_execute_reports_fused() {
    // 六条删除越过熔断，另有一条双方各改的冲突：熔断冻结里的冲突同样先保全。
    let mode = SyncMode::Bidirectional;
    let mut input = fixture::deletion_input(6, 60);
    let path = VaultPath::parse("notes/40.md").expect("固定路径合法");
    let canonical = Presence::Present(fixture::tagged(40, 2, 99));
    let mirror = Presence::Present(fixture::tagged(40, 3, 98));
    input.canonical.entries.insert(path.clone(), canonical);
    input.mirror.entries.insert(path, mirror);
    let decision = plan(input.clone(), None).expect("固定夹具必须可规划");
    let Decision::FuseBlocked(blocked) = decision else {
        panic!("六条删除必须触发熔断");
    };
    assert_eq!(blocked.conflicts.len(), 1);
    let mut ports = FakePorts::healthy(mode);
    ports.input = input;
    ports.execute_result = Err(PlaneError::Fused);
    let outcome = run_round_with(&mut ports, mode, NOW);
    assert_eq!(outcome, Err(PlaneError::Fused));
    let tail = &ports.calls[ports.calls.len() - 2..];
    assert_eq!(tail, &[Call::Preserve, Call::Execute]);
}

#[test]
fn a_failed_preservation_stops_the_round_before_execute() {
    let mode = SyncMode::Bidirectional;
    let mut ports = FakePorts::healthy(mode);
    ports.input = fixture::conflict_input(1);
    ports.preserve_result = Err(PlaneError::Unavailable);
    let outcome = run_round_with(&mut ports, mode, NOW);
    assert_eq!(outcome, Err(PlaneError::Unavailable));
    assert_eq!(ports.count(&Call::Preserve), 1);
    assert_eq!(ports.count(&Call::Execute), 0);
}

#[test]
fn conflict_key_is_stable_for_one_conflict_and_changes_with_either_side_or_the_link() {
    let conflict = first_conflict(&decision(1));
    let key = conflict_key(&LINK, &conflict);
    assert_eq!(key, conflict_key(&LINK, &conflict.clone()));
    let hex = key.chars().all(|c| c.is_ascii_hexdigit());
    assert!(hex && key.len() == 64 && key == key.to_lowercase());
    let mut canonical = conflict.clone();
    canonical.observed.canonical = Presence::Present(fixture::tagged(0, 9, 90));
    assert_ne!(conflict_key(&LINK, &canonical), key);
    let mut mirror = conflict.clone();
    mirror.observed.mirror = Presence::Missing;
    assert_ne!(conflict_key(&LINK, &mirror), key);
    assert_ne!(conflict_key(&[0x12; 16], &conflict), key);
}

#[test]
fn stamps_are_assigned_once_kept_across_rounds_and_pruned_when_a_conflict_goes_away() {
    let three = decision(3);
    let mut stamps = BTreeMap::new();
    assert_eq!(assign_stamps(&LINK, &three, &mut stamps, NOW), Ok(3));
    assert_eq!(stamps.len(), 3);
    for stamp in stamps.values() {
        let digits = stamp.bytes().all(|byte| byte.is_ascii_digit());
        assert!(digits && stamp.len() == 12);
    }
    // 下一轮同一批冲突：不配新戳、旧戳不变（副本路径与幂等键因此不变）。
    let first = stamps.clone();
    assert_eq!(assign_stamps(&LINK, &three, &mut stamps, NOW + 300), Ok(0));
    assert_eq!(stamps, first);
    // 第三条冲突不在了：它的戳回收，其余两条照旧。
    let two = decision(2);
    assert_eq!(assign_stamps(&LINK, &two, &mut stamps, NOW + 600), Ok(0));
    assert_eq!(stamps.len(), 2);
    let kept = stamps
        .iter()
        .all(|(key, stamp)| first.get(key) == Some(stamp));
    assert!(kept);
}

#[test]
fn more_than_512_conflicts_only_stamp_the_first_512_in_path_order() {
    let many = decision(513);
    let mut stamps = BTreeMap::new();
    assert_eq!(assign_stamps(&LINK, &many, &mut stamps, NOW), Ok(512));
    assert_eq!(stamps.len(), 512);
    // 早于纪元的钟配不出戳：失败关闭，不写半截。
    let mut empty = BTreeMap::new();
    assert_eq!(
        assign_stamps(&LINK, &decision(1), &mut empty, -1),
        Err(PlaneError::Unavailable)
    );
    assert!(empty.is_empty());
}
