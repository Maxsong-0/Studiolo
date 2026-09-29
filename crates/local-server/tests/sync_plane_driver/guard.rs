//! 排除项安全顺序（评审 P2-3）：改审批 → 守卫拦住 → 停 human → purge → 清 headless 排除 → 起 human。
//! 审批去掉、headless 仍排除的文件夹算 `pending_purge`：有残留即 `ResidueBlocked`，残留已清但 headless
//! 仍排除即 `ConfigMismatch`，两种都不跑 RunOnce；每轮都把 headless 现状落盘，供 purge 核对。

use super::NOW;
use super::fake_ports::{Call, FakePorts};
use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::driver::{RoundOutcome, run_round_with};
use studiolo_sync_transport::owned_run::SyncMode;

const MODE: SyncMode = SyncMode::MirrorRemote;

fn folders(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// 审批刚去掉 `School`：上次核验集合与 headless 仍含它，审批母本只剩 `private`。
fn approval_dropped_school() -> FakePorts {
    let mut ports = FakePorts::healthy(MODE);
    ports.state.verified_excluded = folders(&["School", "private"]);
    ports.headless_excluded = folders(&["School", "private"]);
    ports
}

#[test]
fn approval_dropped_a_folder_that_headless_still_excludes_and_residue_blocks_before_run_once() {
    let mut ports = approval_dropped_school();
    ports.residue = true;
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert_eq!(outcome, Err(PlaneError::ResidueBlocked));
    assert_eq!(ports.count(&Call::RunOnce), 0);
    assert_eq!(
        ports.calls,
        vec![Call::State, Call::Inspect(MODE), Call::Save, Call::Residue]
    );
    // 守卫拦住的轮次也记下 headless 仍排除 School：purge 凭它放行。
    let saved = ports.saved.last().expect("headless 现状必须落盘");
    assert_eq!(saved.headless_excluded, folders(&["School", "private"]));
    assert_eq!(saved.verified_excluded, folders(&["School", "private"]));
}

#[test]
fn residue_already_purged_but_headless_still_excludes_the_folder_is_a_config_mismatch() {
    let mut ports = approval_dropped_school();
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert_eq!(outcome, Err(PlaneError::ConfigMismatch));
    assert_eq!(ports.count(&Call::RunOnce), 0);
    assert_eq!(ports.count(&Call::Residue), 1);
    // 上次核验集合不动：下一轮守卫仍会查 School，直到 headless 侧也清掉它。
    let both = folders(&["School", "private"]);
    assert_eq!(ports.state.verified_excluded, both);
}

#[test]
fn headless_cleared_too_and_no_residue_runs_a_normal_round_and_forgets_the_folder() {
    let mut ports = approval_dropped_school();
    ports.state.headless_excluded = folders(&["School", "private"]);
    ports.headless_excluded = folders(&["private"]);
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert!(
        matches!(outcome, Ok(RoundOutcome::Settled { .. })),
        "{outcome:?}"
    );
    assert_eq!(ports.count(&Call::RunOnce), 1);
    assert_eq!(ports.state.verified_excluded, folders(&["private"]));
    assert_eq!(ports.state.headless_excluded, folders(&["private"]));
}

#[test]
fn headless_cleared_before_the_approval_changed_is_a_mismatch_and_keeps_the_recorded_state() {
    // 错序：审批还含 School，headless 已先清掉它——四种组合都不一致，本轮不跑，也不改记下的现状。
    let both = folders(&["School", "private"]);
    let mut ports = FakePorts::healthy(MODE);
    ports.excluded = both.clone();
    ports.state.verified_excluded = both.clone();
    ports.state.headless_excluded = both.clone();
    ports.headless_excluded = folders(&["private"]);
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert_eq!(outcome, Err(PlaneError::ConfigMismatch));
    assert_eq!(ports.calls, vec![Call::State, Call::Inspect(MODE)]);
    assert_eq!(ports.state.headless_excluded, both);
}

#[test]
fn only_the_mode_differs_while_a_purge_is_pending_still_waits_for_the_headless_side() {
    // 并集母本在另一模式下命中：模式问题交 SetMode，但待清文件夹仍在，先按安全顺序停住。
    let mut ports = approval_dropped_school();
    ports.current_mode = SyncMode::Bidirectional;
    let outcome = run_round_with(&mut ports, MODE, NOW);
    assert_eq!(outcome, Err(PlaneError::ConfigMismatch));
    assert_eq!(ports.count(&Call::SetMode(MODE)), 0);
    assert_eq!(ports.count(&Call::RunOnce), 0);
}
