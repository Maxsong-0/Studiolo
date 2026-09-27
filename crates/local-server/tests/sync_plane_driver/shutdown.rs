//! 关停停点（评审 P3-6）：关停标志一旦为真，本轮在最近的安全停点以 `Stopping` 结束——不再发起新的
//! 运行、观察或执行；正在跑的 RunOnce 被 `request_stop(OwnerRequested)` 停下后先按现状回收落盘。

use super::NOW;
use super::fake_ports::{Call, FakePorts};
use super::fixture;
use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::driver::run_round_with;
use studiolo_sync_transport::owned_run::{StopReason, SyncMode};

const MODE: SyncMode = SyncMode::MirrorRemote;

#[test]
fn a_stop_before_inspect_ends_the_round_after_reading_state_only() {
    let mut ports = FakePorts::healthy(MODE);
    ports.stop_after = Some(1);
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert_eq!(outcome, Err(PlaneError::Stopping));
    assert_eq!(ports.calls, vec![Call::State]);
    assert!(ports.saved.is_empty());
}

#[test]
fn a_run_once_stopped_by_the_owner_is_reaped_and_saved_before_stopping() {
    // State、Inspect、Residue、NextSequence、Save 之后第 6 次调用是 RunOnce：此时关停标志变真。
    let mut ports = FakePorts::healthy(MODE);
    ports.stop_after = Some(6);
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert_eq!(outcome, Err(PlaneError::Stopping));
    assert_eq!(ports.count(&Call::RunOnce), 1);
    assert_eq!(ports.count(&Call::Quiescent), 0);
    assert_eq!(ports.calls.last(), Some(&Call::Save));
    // 回收已落盘：不再是「已启动」，按非干净退出记退避，下次启动走强制恢复。
    let reaped = ports.saved.last().expect("回收后必有保存");
    assert!(!reaped.last_run_clean);
    assert_eq!(reaped.started_sequence, None);
    assert_eq!(reaped.not_before_unix, Some(NOW + 5));
}

#[test]
fn a_stop_before_observation_neither_observes_nor_executes() {
    // 取票与续做守卫之后（第 9 次调用）变真：停在观察之前。
    let mut ports = FakePorts::healthy(MODE);
    ports.stop_after = Some(9);
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert_eq!(outcome, Err(PlaneError::Stopping));
    assert_eq!(ports.count(&Call::Quiescent), 1);
    assert_eq!(ports.count(&Call::Unresolved), 1);
    assert_eq!(ports.count(&Call::Observe(MODE)), 0);
    assert_eq!(ports.count(&Call::Execute), 0);
    // 观察之后才变真：停在执行之前。
    let mut ports = FakePorts::healthy(MODE);
    ports.stop_after = Some(10);
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert_eq!(outcome, Err(PlaneError::Stopping));
    assert_eq!(ports.count(&Call::Observe(MODE)), 1);
    assert_eq!(ports.count(&Call::Execute), 0);
}

#[test]
fn a_stop_before_set_mode_does_not_switch_the_headless_mode() {
    let mut ports = FakePorts::healthy(SyncMode::Bidirectional);
    ports.stop_after = Some(3);
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert_eq!(outcome, Err(PlaneError::Stopping));
    assert_eq!(ports.count(&Call::SetMode(MODE)), 0);
    assert_eq!(ports.current_mode, SyncMode::Bidirectional);
}

#[test]
fn only_an_owner_requested_stop_turns_an_unclean_run_into_stopping() {
    let mut run = fixture::headless(143);
    assert_eq!(run.failure(), PlaneError::Transport);
    run.observation.stop_reason = Some(StopReason::DeadlineExceeded);
    assert_eq!(run.failure(), PlaneError::Transport);
    run.observation.stop_reason = Some(StopReason::OwnerRequested);
    assert_eq!(run.failure(), PlaneError::Stopping);
}
