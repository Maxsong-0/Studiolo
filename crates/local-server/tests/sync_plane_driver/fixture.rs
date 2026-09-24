//! 驱动契约的纯数据夹具：持久小状态、headless 运行结果与可被纯规划器接受的冻结输入。

use std::collections::{BTreeMap, BTreeSet};
use studiolo_core::VaultPath;
use studiolo_local_server::sync_plane::driver::HeadlessRun;
use studiolo_local_server::sync_plane::recovery::PlaneState;
use studiolo_sync_bridge::{
    ApprovedScope, Baseline, Binding, FileVersion, Input, Inventory, Limits, Pair, PlanningCursor,
    Presence, Side,
};
use studiolo_sync_transport::owned_run::{
    ExitObservation, Quiescence, RunId, RunObservation, RunPhase,
};

/// 首次运行的持久小状态：上次干净、没有已启动序号、没有退避。
pub fn first_run_state() -> PlaneState {
    PlaneState {
        version: 1,
        last_run_clean: true,
        started_sequence: None,
        last_exit_unix: None,
        consecutive_rc1: 0,
        not_before_unix: None,
        verified_excluded: Vec::new(),
        applied_mode: None,
        checksum: String::new(),
    }
}

/// 一次已闭合的 headless 运行；`exit` 决定 clean／rc_one 的分类。
pub fn headless(exit: i32) -> HeadlessRun {
    let observation = RunObservation {
        phase: RunPhase::Closed,
        quiescence: Quiescence::Confirmed,
        exit: ExitObservation::Code(exit),
        stop_reason: None,
    };
    HeadlessRun {
        run_id: RunId::from_bytes([0x5A; 16]),
        observation,
        clean: exit == 0,
        rc_one: exit == 1,
    }
}

/// 两个根 token 不同、其余固定的绑定。
pub fn binding() -> Binding {
    Binding {
        link_id: [0x11; 16],
        canonical_vault: [0x22; 16],
        remote_vault: [0x33; 16],
        canonical_root_token: [0x44; 32],
        mirror_root_token: [0x55; 32],
        scope_hash: [0x66; 32],
        scope_epoch: 1,
    }
}

fn version(index: u64) -> FileVersion {
    let mut tag = [0u8; 32];
    tag[..8].copy_from_slice(&index.to_be_bytes());
    tag[8] = 1;
    FileVersion {
        revision: tag,
        sha256: tag,
        bytes: 10 + index,
    }
}

fn inventory(side: Side, sequence: u64, entries: BTreeMap<VaultPath, Presence>) -> Inventory {
    Inventory {
        binding: binding(),
        side,
        sequence,
        complete: true,
        entries,
    }
}

/// 没有任何路径、没有基线的冻结输入：规划结果为空，既无动作也无冲突。
pub fn empty_input() -> Input {
    deletion_input(0, 0)
}

/// 原共同基线里有 `live` 条存活路径，本次镜像侧对其中前 `deletions` 条显式缺失：
/// 纯规划器会对这些路径各提一条 canonical 侧条件删除。
pub fn deletion_input(deletions: u64, live: u64) -> Input {
    assert!(deletions <= live, "删除只能来自原共同基线的存活路径");
    let paths: Vec<VaultPath> = (0..live)
        .map(|index| VaultPath::parse(format!("notes/{index}.md")).expect("固定路径合法"))
        .collect();
    let mut baseline_entries = BTreeMap::new();
    let mut canonical = BTreeMap::new();
    let mut mirror = BTreeMap::new();
    for (index, path) in paths.iter().enumerate() {
        let index = index as u64;
        baseline_entries.insert(
            path.clone(),
            Pair {
                canonical: Presence::Present(version(index)),
                mirror: Presence::Present(version(index)),
            },
        );
        canonical.insert(path.clone(), Presence::Present(version(index)));
        let on_mirror = if index < deletions {
            Presence::Missing
        } else {
            Presence::Present(version(index))
        };
        mirror.insert(path.clone(), on_mirror);
    }
    let established = live > 0;
    let observation = u64::from(established);
    Input {
        scope: ApprovedScope {
            binding: binding(),
            paths: paths.iter().cloned().collect::<BTreeSet<_>>(),
        },
        baseline: Baseline {
            binding: binding(),
            version: observation,
            canonical_observation: observation,
            mirror_observation: observation,
            entries: baseline_entries,
        },
        canonical: inventory(Side::Canonical, observation + 1, canonical),
        mirror: inventory(Side::Mirror, observation + 1, mirror),
        cursor: PlanningCursor {
            binding: binding(),
            baseline_version: observation,
            last_plan_sequence: 0,
            last_canonical_observation: observation,
            last_mirror_observation: observation,
        },
        expected_baseline_version: observation,
        plan_sequence: 1,
        limits: Limits::conservative(1 << 20),
    }
}
