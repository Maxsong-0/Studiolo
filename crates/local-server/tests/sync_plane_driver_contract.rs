//! 驱动循环契约（T-132 P1 W3）：用假端口证明设计文档第 2 节的顺序、恢复／退避、守卫、
//! 未知终态不重放与熔断停点，并证明 `deletion_fuse_blocked` 与纯规划器同结论。不碰真实 headless。

#[path = "sync_plane_driver/fake_ports.rs"]
mod fake_ports;
#[path = "sync_plane_driver/fixture.rs"]
mod fixture;
#[path = "sync_plane_driver/reconcile.rs"]
mod reconcile;
#[path = "sync_plane_driver/reconcile_support.rs"]
mod reconcile_support;

use fake_ports::{Call, FakePorts};
use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::driver::{
    RoundOutcome, deletion_fuse_blocked, run_round_with,
};
use studiolo_sync_bridge::{Decision, plan};
use studiolo_sync_transport::owned_run::SyncMode;

const NOW: i64 = 1_790_000_000;

#[test]
fn normal_round_runs_inspect_guard_run_once_quiescence_observe_execute() {
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);
    assert_eq!(
        outcome,
        Ok(RoundOutcome::Settled {
            sequence: 1,
            baseline_advanced: true,
        })
    );
    assert_eq!(
        ports.calls,
        vec![
            Call::State,
            Call::Inspect(SyncMode::MirrorRemote),
            Call::Residue,
            Call::NextSequence,
            Call::Save,
            Call::RunOnce,
            Call::Save,
            Call::Quiescent,
            Call::Unresolved,
            Call::Observe(SyncMode::MirrorRemote),
            Call::Execute,
        ]
    );
    // 「已启动」先持久（带序号），回收后清掉并记干净。
    assert_eq!(ports.saved[0].started_sequence, Some(7));
    assert_eq!(ports.saved[1].started_sequence, None);
    assert!(ports.saved[1].last_run_clean);
    assert_eq!(ports.saved[1].not_before_unix, None);
    assert_eq!(ports.saved[1].applied_mode, Some(SyncMode::MirrorRemote));
    assert_eq!(ports.saved[1].verified_excluded, vec!["private".to_owned()]);
}

#[test]
fn unclean_previous_run_forces_mirror_remote_set_mode_before_run_once() {
    let mut ports = FakePorts::healthy(SyncMode::Bidirectional);
    ports.state.last_run_clean = false;
    let outcome = run_round_with(&mut ports, SyncMode::Bidirectional, NOW);
    // 强制恢复轮只跑 SetMode＋RunOnce 并干净回收，本轮不取票、不观察、不执行（评审 P2-1）。
    assert!(matches!(outcome, Ok(RoundOutcome::Recovered)));
    assert_eq!(
        ports.calls,
        vec![
            Call::State,
            Call::Inspect(SyncMode::MirrorRemote),
            Call::Residue,
            Call::SetMode(SyncMode::MirrorRemote),
            Call::Inspect(SyncMode::MirrorRemote),
            Call::NextSequence,
            Call::Save,
            Call::RunOnce,
            Call::Save,
        ]
    );
    assert_eq!(ports.state.applied_mode, Some(SyncMode::MirrorRemote));
    assert!(ports.state.last_run_clean);
    // 断电留下的「已启动」序号同样触发强制恢复。
    let mut ports = FakePorts::healthy(SyncMode::Bidirectional);
    ports.state.started_sequence = Some(3);
    let _ = run_round_with(&mut ports, SyncMode::Bidirectional, NOW);
    assert_eq!(ports.calls[3], Call::SetMode(SyncMode::MirrorRemote));
}

#[test]
fn consecutive_rc_one_backs_off_5_10_20_and_skips_inside_the_window() {
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    ports.run_once_exit = 1;
    let mut now = NOW;
    let mut waits = Vec::new();
    for expected_count in 1..=3 {
        let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, now);
        assert_eq!(outcome, Err(PlaneError::Transport));
        let saved = ports.saved.last().expect("回收后必有保存");
        assert_eq!(saved.consecutive_rc1, expected_count);
        assert!(!saved.last_run_clean);
        let until = saved.not_before_unix.expect("rc=1 必记退避");
        waits.push(until - now);
        now = until;
    }
    assert_eq!(waits, vec![5, 10, 20]);
    // 退避窗口内：什么都不跑，连 InspectConfig 都不发。
    let before = ports.calls.len();
    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, now - 1);
    assert_eq!(outcome, Ok(RoundOutcome::Skipped));
    assert_eq!(&ports.calls[before..], &[Call::State]);
    // 窗口一过就恢复；退出码非 0 非 1 只等最短间隔 5 s 且清零连续计数。
    ports.run_once_exit = 2;
    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, now);
    assert_eq!(outcome, Err(PlaneError::Transport));
    let saved = ports.saved.last().expect("回收后必有保存");
    assert_eq!(saved.consecutive_rc1, 0);
    assert_eq!(saved.not_before_unix, Some(now + 5));
}

#[test]
fn config_mismatch_stops_before_run_once() {
    // 模式不符：SetMode 之后复核仍不一致（假端口的排除项不一致）→ ConfigMismatch。
    let mut ports = FakePorts::healthy(SyncMode::Bidirectional);
    ports.config_consistent = false;
    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);
    assert_eq!(outcome, Err(PlaneError::ConfigMismatch));
    assert_eq!(ports.count(&Call::RunOnce), 0);
    assert_eq!(ports.count(&Call::Save), 0);
    // 只有模式不符且 SetMode 后一致：正常继续。
    let mut ports = FakePorts::healthy(SyncMode::Bidirectional);
    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);
    assert!(matches!(outcome, Ok(RoundOutcome::Settled { .. })));
    assert_eq!(ports.count(&Call::SetMode(SyncMode::MirrorRemote)), 1);
    assert_eq!(ports.count(&Call::Inspect(SyncMode::MirrorRemote)), 2);
}

#[test]
fn residue_guard_blocks_the_round_before_run_once() {
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    ports.residue = true;
    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);
    assert_eq!(outcome, Err(PlaneError::ResidueBlocked));
    assert_eq!(
        ports.calls,
        vec![
            Call::State,
            Call::Inspect(SyncMode::MirrorRemote),
            Call::Residue,
        ]
    );
}

#[test]
fn unknown_outcome_is_reported_and_not_replayed_next_round() {
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    ports.execute_result = Ok(RoundOutcome::Unknown { sequence: 4 });
    let first = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);
    assert_eq!(first, Ok(RoundOutcome::Unknown { sequence: 4 }));
    let second = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW + 300);
    assert_eq!(second, Ok(RoundOutcome::Unknown { sequence: 4 }));
    // 第二轮 headless 照跑，但停在原键核对守卫：不观察、不执行。
    assert_eq!(ports.count(&Call::RunOnce), 2);
    assert_eq!(ports.count(&Call::Execute), 1);
    assert_eq!(ports.count(&Call::Observe(SyncMode::MirrorRemote)), 1);
}

#[test]
fn fused_reservation_stops_without_preresolve() {
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    ports.execute_result = Err(PlaneError::Fused);
    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);
    assert_eq!(outcome, Err(PlaneError::Fused));
    assert_eq!(ports.count(&Call::Preresolve), 0);
    assert_eq!(ports.count(&Call::Execute), 1);
    // 没有静止证据同样停在执行之前。
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    ports.quiescent = false;
    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);
    assert_eq!(outcome, Err(PlaneError::Quiescence));
    assert_eq!(ports.count(&Call::Observe(SyncMode::MirrorRemote)), 0);
}

#[test]
fn deletion_fuse_matches_the_planner_on_the_grid() {
    for live in [0_u64, 10, 60] {
        for deletions in 0..=7_u64 {
            let blocked = deletion_fuse_blocked(deletions, live);
            if deletions > live {
                // 存活路径为 0（或不足）时构造不出对应计划；纯规划器同一判据下任何删除都过比例门槛。
                assert!(blocked, "deletions={deletions} live={live}");
                continue;
            }
            let decision = plan(fixture::deletion_input(deletions, live), None)
                .unwrap_or_else(|error| panic!("固定夹具必须可规划：{error:?}"));
            let planner_blocked = match decision {
                Decision::FuseBlocked(blocked) => {
                    assert_eq!(blocked.stats.canonical, deletions);
                    assert_eq!(blocked.stats.baseline_live_paths, live);
                    true
                }
                Decision::Ready(plan) => {
                    assert_eq!(plan.stats().canonical, deletions);
                    assert_eq!(plan.stats().baseline_live_paths, live);
                    false
                }
            };
            assert_eq!(
                blocked, planner_blocked,
                "deletions={deletions} live={live}"
            );
        }
    }
    // 等号可过：恰好 5 条、恰好 10%。
    assert!(!deletion_fuse_blocked(5, 100));
    assert!(!deletion_fuse_blocked(1, 10));
    assert!(deletion_fuse_blocked(6, 1000));
    assert!(deletion_fuse_blocked(2, 10));
    assert!(deletion_fuse_blocked(u64::MAX, u64::MAX));
}
