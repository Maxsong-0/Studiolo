//! BUG-046 删除守卫：主库没开文件操作时，新批次里只要有主库删除就整批暂停——不预解决、不保全、
//! 不交执行段，也就不会留下解不开的熔断；上一批没闭合照常交执行段续做，只跳过会当场删除的预解决；
//! 开了文件操作一切照旧。守卫每次的结论（被挡的个数，放行为 0）交健康节。

use super::NOW;
use super::fake_ports::{Call, FakePorts};
use super::fixture;
use studiolo_core::VaultPath;
use studiolo_local_server::sync_plane::PlaneError;
use studiolo_local_server::sync_plane::driver::{
    RoundOutcome, canonical_deletions, run_round_with,
};
use studiolo_sync_bridge::{Input, Presence, plan};
use studiolo_sync_transport::owned_run::SyncMode;

const MODES: [SyncMode; 2] = [SyncMode::MirrorRemote, SyncMode::Bidirectional];

fn path(index: u64) -> VaultPath {
    VaultPath::parse(format!("notes/{index}.md")).expect("固定路径合法")
}

/// 前 `conflicts` 条镜像侧删了、canonical 却改过：规划成「删除对改动」冲突，
/// 镜像为准时预解决就是删掉 canonical。
fn delete_conflict_input(conflicts: u64) -> Input {
    let mut input = fixture::deletion_input(conflicts, 60);
    for index in 0..conflicts {
        let changed = Presence::Present(fixture::tagged(index, 2, 50 + index));
        input.canonical.entries.insert(path(index), changed);
    }
    input
}

/// 前 `deletes` 条是镜像侧删了的普通删除，紧接着 `edits` 条两侧都改过（冲突，但预解决不删除）。
fn deletes_with_edit_conflicts(deletes: u64, edits: u64) -> Input {
    let mut input = fixture::deletion_input(deletes, 60);
    for index in deletes..deletes + edits {
        let canonical = Presence::Present(fixture::tagged(index, 2, 20 + index));
        let mirror = Presence::Present(fixture::tagged(index, 3, 30 + index));
        input.canonical.entries.insert(path(index), canonical);
        input.mirror.entries.insert(path(index), mirror);
    }
    input
}

fn gated(mode: SyncMode, input: Input) -> FakePorts {
    let mut ports = FakePorts::healthy(mode);
    ports.input = input;
    ports.deletes_enabled = Ok(false);
    ports
}

fn tail(ports: &FakePorts, len: usize) -> &[Call] {
    &ports.calls[ports.calls.len() - len..]
}

#[test]
fn new_batches_with_canonical_deletes_pause_whole_when_file_operations_are_off() {
    for mode in MODES {
        let mut ports = gated(mode, fixture::deletion_input(1, 60));

        let outcome = run_round_with(&mut ports, mode, NOW);

        assert_eq!(
            outcome,
            Ok(RoundOutcome::DeletesBlocked { count: 1 }),
            "{mode:?}"
        );
        assert_eq!(ports.count(&Call::Execute), 0, "{mode:?}");
        assert_eq!(ports.count(&Call::Preresolve), 0, "{mode:?}");
        assert_eq!(ports.count(&Call::Preserve), 0, "{mode:?}");
        assert_eq!(
            tail(&ports, 3),
            &[Call::Observe(mode), Call::DeletesEnabled, Call::Pending]
        );
        assert_eq!(ports.delete_gate, [1], "{mode:?}");
    }
}

#[test]
fn fuse_sized_delete_batches_pause_before_the_fuse_can_be_persisted() {
    // 六条删除本会触发熔断（熔断一旦持久就没有解除入口）；删不了的批次在交执行段之前就停下。
    let input = fixture::deletion_input(6, 60);
    assert!(matches!(
        plan(input.clone(), None),
        Ok(studiolo_sync_bridge::Decision::FuseBlocked(_))
    ));
    let mut ports = gated(SyncMode::MirrorRemote, input);
    ports.execute_result = Err(PlaneError::Fused);

    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);

    assert_eq!(outcome, Ok(RoundOutcome::DeletesBlocked { count: 6 }));
    assert_eq!(ports.count(&Call::Execute), 0);
    assert_eq!(ports.delete_gate, [6]);
}

#[test]
fn mirror_remote_delete_conflicts_pause_before_any_preservation() {
    let mut ports = gated(SyncMode::MirrorRemote, delete_conflict_input(2));

    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);

    assert_eq!(outcome, Ok(RoundOutcome::DeletesBlocked { count: 2 }));
    assert_eq!(ports.count(&Call::Preresolve), 0, "保全副本也不能写");
    assert_eq!(ports.count(&Call::Execute), 0);
    assert_eq!(ports.delete_gate, [2]);
    // 双向模式不预解决冲突，同一输入里没有要做的主库删除：守卫不查、照常保全并执行。
    let mut ports = gated(SyncMode::Bidirectional, delete_conflict_input(2));

    let outcome = run_round_with(&mut ports, SyncMode::Bidirectional, NOW);

    assert!(
        matches!(outcome, Ok(RoundOutcome::Settled { .. })),
        "{outcome:?}"
    );
    assert_eq!(ports.count(&Call::DeletesEnabled), 0);
    assert_eq!(ports.count(&Call::Preserve), 1);
    assert_eq!(ports.count(&Call::Execute), 1);
    assert_eq!(ports.delete_gate, [0]);
}

#[test]
fn an_unsettled_batch_is_still_resumed_by_execute_with_file_operations_off() {
    // 执行段续做旧批次用的是冻结的旧输入，本轮规划出的删除这一轮不发；旧批次里没发出的删除会在接缝被
    // 确定拒成无效果，批次照常闭合，守卫挡它只会再卡死一次。守卫照报被挡的个数：旧批次一闭合就要暂停。
    // 镜像为准的删除冲突：预解决会当场删除、被拒、整轮报错，那就永远走不到续做，所以只跳过它。
    for (mode, input, blocked, tail_calls) in [
        (
            SyncMode::MirrorRemote,
            fixture::deletion_input(1, 60),
            1,
            vec![Call::DeletesEnabled, Call::Pending, Call::Execute],
        ),
        (
            SyncMode::Bidirectional,
            fixture::deletion_input(1, 60),
            1,
            vec![Call::DeletesEnabled, Call::Pending, Call::Execute],
        ),
        (
            SyncMode::MirrorRemote,
            delete_conflict_input(2),
            2,
            vec![Call::DeletesEnabled, Call::Pending, Call::Execute],
        ),
        // 预解决不删除的冲突、双向的保全都和改动前一样照走：只有当场删除的那一步才要跳过。
        (
            SyncMode::MirrorRemote,
            deletes_with_edit_conflicts(1, 2),
            1,
            vec![
                Call::DeletesEnabled,
                Call::Pending,
                Call::Preresolve,
                Call::Execute,
            ],
        ),
        (
            SyncMode::Bidirectional,
            deletes_with_edit_conflicts(1, 2),
            1,
            vec![
                Call::DeletesEnabled,
                Call::Pending,
                Call::Preserve,
                Call::Execute,
            ],
        ),
    ] {
        let mut ports = gated(mode, input);
        ports.pending = true;

        let outcome = run_round_with(&mut ports, mode, NOW);

        assert!(
            matches!(outcome, Ok(RoundOutcome::Settled { .. })),
            "{mode:?} {outcome:?}"
        );
        assert_eq!(ports.count(&Call::Execute), 1, "{mode:?}");
        assert_eq!(
            tail(&ports, tail_calls.len()),
            tail_calls.as_slice(),
            "{mode:?}"
        );
        assert_eq!(ports.delete_gate, [blocked], "{mode:?}");
    }
}

#[test]
fn with_file_operations_on_deletes_go_to_execute_as_before() {
    for mode in MODES {
        let mut ports = FakePorts::healthy(mode);
        ports.input = fixture::deletion_input(1, 60);

        let outcome = run_round_with(&mut ports, mode, NOW);

        assert!(
            matches!(outcome, Ok(RoundOutcome::Settled { .. })),
            "{mode:?}"
        );
        assert_eq!(ports.count(&Call::DeletesEnabled), 1, "{mode:?}");
        assert_eq!(ports.count(&Call::Pending), 0, "开了就不用看有没有旧批次");
        assert_eq!(ports.count(&Call::Execute), 1, "{mode:?}");
        assert_eq!(ports.delete_gate, [0], "{mode:?}");
    }
    // 镜像为准的删除冲突：开着文件操作就照常预解决（删掉 canonical 由文件操作协议做）。
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    ports.input = delete_conflict_input(2);
    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);
    assert!(
        matches!(outcome, Ok(RoundOutcome::Settled { .. })),
        "{outcome:?}"
    );
    assert_eq!(ports.count(&Call::Preresolve), 1);
    assert_eq!(ports.count(&Call::Pending), 0);
    assert_eq!(ports.delete_gate, [0]);
    // 没有主库删除就不查：每轮都锁写者问一次没有必要；守卫照样报「放行」。
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    let _ = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);
    assert_eq!(ports.count(&Call::Pending), 0);
    assert_eq!(ports.count(&Call::DeletesEnabled), 0);
    assert_eq!(ports.delete_gate, [0]);
}

#[test]
fn an_unreadable_switch_fails_closed_before_execute() {
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    ports.input = fixture::deletion_input(1, 60);
    ports.deletes_enabled = Err(PlaneError::Unavailable);

    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);

    assert_eq!(outcome, Err(PlaneError::Unavailable));
    assert_eq!(ports.count(&Call::Execute), 0);
    assert!(ports.delete_gate.is_empty(), "查不出就不报结论");
}

#[test]
fn rounds_that_never_reach_the_gate_report_nothing() {
    // 上一批仍有未知终态：本轮不观察，守卫没跑，健康节沿用上次证明过的值。
    let mut ports = FakePorts::healthy(SyncMode::MirrorRemote);
    ports.input = fixture::deletion_input(1, 60);
    ports.deletes_enabled = Ok(false);
    ports.unresolved = Some(3);

    let outcome = run_round_with(&mut ports, SyncMode::MirrorRemote, NOW);

    assert_eq!(outcome, Ok(RoundOutcome::Unknown { sequence: 3 }));
    assert!(ports.delete_gate.is_empty());
    assert_eq!(ports.count(&Call::DeletesEnabled), 0);
}

#[test]
fn canonical_deletions_counts_planned_deletes_and_mirror_remote_preresolve_deletes() {
    let count = |input: Input, mode| {
        canonical_deletions(&plan(input, None).expect("固定夹具必须可规划"), mode)
    };
    for mode in MODES {
        assert_eq!(count(fixture::empty_input(), mode), 0);
        assert_eq!(
            count(fixture::conflict_input(2), mode),
            0,
            "两侧都改不是删除"
        );
        assert_eq!(count(fixture::deletion_input(3, 60), mode), 3);
        assert_eq!(
            count(fixture::deletion_input(6, 60), mode),
            6,
            "熔断冻结的批次照样算"
        );
        assert_eq!(count(deletes_with_edit_conflicts(1, 2), mode), 1);
    }
    assert_eq!(count(delete_conflict_input(2), SyncMode::MirrorRemote), 2);
    assert_eq!(count(delete_conflict_input(2), SyncMode::Bidirectional), 0);
}
