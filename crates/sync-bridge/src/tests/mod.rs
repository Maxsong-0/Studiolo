//! 仅纯契约草稿；未运行，不是文件系统/同步/持久恢复验收。

use super::*;
use std::collections::{BTreeMap, BTreeSet};
use studiolo_core::VaultPath;

mod agreement;
mod budgets;
mod deletion;
mod evidence;
mod retries;

fn path(number: usize) -> VaultPath {
    VaultPath::parse(format!("项目/验收/资料-{number:04}.bin")).unwrap()
}

fn file(revision: u8, hash: u8, bytes: u64) -> Presence {
    Presence::Present(FileVersion {
        revision: [revision; 32],
        sha256: [hash; 32],
        bytes,
    })
}

fn present(value: u8) -> Presence {
    file(value, value, 10)
}

fn binding() -> Binding {
    Binding {
        link_id: [1; 16],
        canonical_vault: [2; 16],
        remote_vault: [3; 16],
        canonical_root_token: [4; 32],
        mirror_root_token: [5; 32],
        scope_hash: [6; 32],
        scope_epoch: 1,
    }
}

fn input(base: Option<Pair>, canonical: Presence, mirror: Presence) -> Input {
    let bound = binding();
    let previous = if base.is_some() { 1 } else { 0 };
    let mut entries = BTreeMap::new();
    if let Some(base) = base {
        entries.insert(path(0), base);
    }
    Input {
        scope: ApprovedScope {
            binding: bound.clone(),
            paths: BTreeSet::from([path(0)]),
        },
        baseline: Baseline {
            binding: bound.clone(),
            version: previous,
            canonical_observation: previous,
            mirror_observation: previous,
            entries,
        },
        canonical: Inventory {
            binding: bound.clone(),
            side: Side::Canonical,
            sequence: previous + 1,
            complete: true,
            entries: BTreeMap::from([(path(0), canonical)]),
        },
        mirror: Inventory {
            binding: bound.clone(),
            side: Side::Mirror,
            sequence: previous + 1,
            complete: true,
            entries: BTreeMap::from([(path(0), mirror)]),
        },
        cursor: PlanningCursor {
            binding: bound,
            baseline_version: previous,
            last_plan_sequence: previous,
            last_canonical_observation: previous,
            last_mirror_observation: previous,
        },
        expected_baseline_version: previous,
        plan_sequence: previous + 1,
        limits: Limits::conservative(1000),
    }
}

fn common(value: Presence) -> Pair {
    Pair {
        canonical: value.clone(),
        mirror: value,
    }
}

fn ready(input: Input) -> Plan {
    match plan(input, None).unwrap() {
        Decision::Ready(plan) => *plan,
        Decision::FuseBlocked(_) => panic!("此用例不应触发删除熔断"),
    }
}

fn baseline_paths(count: usize) -> Input {
    assert!(count > 0);
    let mut data = input(Some(common(present(1))), present(1), present(1));
    for number in 1..count {
        data.scope.paths.insert(path(number));
        data.baseline
            .entries
            .insert(path(number), common(present(1)));
        data.canonical.entries.insert(path(number), present(1));
        data.mirror.entries.insert(path(number), present(1));
    }
    data
}
