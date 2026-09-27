//! 第 8 步：两侧完整观察、经 `SyncScope::approve_union` 核准，组成冻结 `Input`。

use super::super::PlaneError;
use super::super::witness::{PlaneWitness, WitnessContext};
use super::content::RoundContext;
use super::evidence::max_object_bytes;
use std::collections::BTreeSet;
use studiolo_observability::quota::SyncUsage;
use studiolo_sync_bridge::{ApprovedScope, Digest, Input, Inventory, Limits};

/// 一次观察的全部产物；`execute` 用它接着同一实例标识与观察序号继续。
pub struct Observed {
    /// 冻结输入。
    pub input: Input,
    /// canonical 完整观察。
    pub canonical: Inventory,
    /// 镜像完整观察。
    pub mirror: Inventory,
    /// 本轮镜像用量（健康页 `sync-quota`）。
    pub usage: SyncUsage,
    /// 打开实例标识。
    pub epoch: Digest,
    /// 下一个 canonical 观察序号。
    pub next_canonical: u64,
    /// 下一个镜像观察序号。
    pub next_mirror: u64,
}

/// 两侧完整观察并经 `approve_union` 核准，组成冻结 `Input`：序号取游标 +1，
/// `Limits::conservative` 的字节上限取范围内最大 `max_bytes`。
///
/// # Errors
/// 任一侧不完整或范围未批准返回 `Observation`；持久状态不可读返回 `Storage`。
pub fn observe_both(context: &RoundContext<'_>) -> Result<Observed, PlaneError> {
    let plane = context.plane;
    let (view_state, epoch) = {
        let bridge = plane.bridge().lock().map_err(|_| PlaneError::Unavailable)?;
        (
            bridge.view().map_err(|_| PlaneError::Storage)?,
            bridge.observation_epoch(),
        )
    };
    let binding = view_state.cursor.binding.clone();
    let mut witness = PlaneWitness::new(
        plane,
        context.view.clone(),
        context.ticket,
        WitnessContext {
            binding: binding.clone(),
            approved: ApprovedScope {
                binding,
                paths: BTreeSet::new(),
            },
            baseline: view_state.baseline.clone(),
            epoch,
            next_canonical: view_state
                .cursor
                .last_canonical_observation
                .saturating_add(1),
            next_mirror: view_state.cursor.last_mirror_observation.saturating_add(1),
        },
    );
    let required: BTreeSet<_> = view_state.baseline.entries.keys().cloned().collect();
    let (canonical, mirror, usage) = witness.observe_pair(&required)?;
    let approved = witness.approve(&canonical, &mirror)?;
    plane.health.record_usage(usage);
    let (next_canonical, next_mirror) = witness.next_sequences();
    let input = Input {
        scope: approved,
        baseline: view_state.baseline.clone(),
        canonical: canonical.clone(),
        mirror: mirror.clone(),
        cursor: view_state.cursor.clone(),
        expected_baseline_version: view_state.baseline.version,
        plan_sequence: view_state.cursor.last_plan_sequence.saturating_add(1),
        limits: plan_limits(plane)?,
    };
    Ok(Observed {
        input,
        canonical,
        mirror,
        usage,
        epoch,
        next_canonical,
        next_mirror,
    })
}

/// 本链冻结的规划预算：保守删除阈值，字节上限取范围内最大 `max_bytes`。
/// `sync-init` 初始化 Bridge 持久状态时必须用同一函数，否则预留会因预算不同被拒。
///
/// # Errors
/// 范围导出失败返回 `Observation`。
pub fn plan_limits(plane: &super::super::SyncPlane) -> Result<Limits, PlaneError> {
    let approval = plane.approval();
    let selection = approval
        .scope()
        .observation_selection(approval.binding())
        .map_err(|_| PlaneError::Observation)?;
    Ok(Limits::conservative(max_object_bytes(&selection)))
}
