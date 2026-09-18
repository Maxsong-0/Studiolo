//! 保守删除熔断复核：先算整批，再决定整批。没有任何按条放行的分支。
//!
//! 阈值沿用主线已冻结的默认：整批删除数 >5，或 >原完整共同基线存活路径数的 10%，
//! 即阻断整个计划；等号可过；整数溢出拒绝。本片不提供熔断解除能力。

use super::error::ExecutorError;
use crate::{Change, DeleteLimits, DeleteStats, FuseReason, OperationKind, Plan, Presence, Side};

/// 整批删除复核结论；只有 `verdict` 为 None 才可能产生删除。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeletionAudit {
    /// 本批两个方向的删除数合计。
    pub deletions: u64,
    /// 原完整共同基线的存活路径数，即比例分母。
    pub baseline_live_paths: u64,
    /// 触发的门槛；None 表示未触发。
    pub verdict: Option<FuseReason>,
}

/// 整批复核通过才铸出的删除放行票据；无票据不得发出任何删除。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeletionClearance {
    deletions: u64,
}

impl DeletionClearance {
    /// 本票据放行的删除总数。
    pub fn deletions(&self) -> u64 {
        self.deletions
    }
}

/// 独立重数整批删除、分母与复制字节，并与冻结统计逐字段对账。
pub fn audit_batch(plan: &Plan) -> Result<DeletionAudit, ExecutorError> {
    let input = plan.frozen_input();
    let mut stats = DeleteStats::default();
    for action in plan.actions() {
        match &action.change {
            Change::CreateOnly { source } | Change::Replace { source, .. } => {
                stats.copy_bytes = stats
                    .copy_bytes
                    .checked_add(source.bytes)
                    .ok_or(ExecutorError::Overflow)?;
            }
            Change::Delete { .. } => {
                let counter = match action.key.target {
                    Side::Canonical => &mut stats.canonical,
                    Side::Mirror => &mut stats.mirror,
                };
                *counter = counter.checked_add(1).ok_or(ExecutorError::Overflow)?;
            }
        }
        if (action.key.operation == OperationKind::Delete)
            != matches!(action.change, Change::Delete { .. })
        {
            return Err(ExecutorError::PlanMismatch);
        }
    }
    for path in &input.scope.paths {
        if input
            .baseline
            .entries
            .get(path)
            .is_some_and(|pair| matches!(pair.canonical, Presence::Present(_)))
        {
            stats.baseline_live_paths = stats
                .baseline_live_paths
                .checked_add(1)
                .ok_or(ExecutorError::Overflow)?;
        }
    }
    // 冻结统计是持久层重算出来的；对不上说明这不是同一批，不能继续。
    if stats != plan.stats() {
        return Err(ExecutorError::PlanMismatch);
    }
    Ok(DeletionAudit {
        deletions: stats
            .canonical
            .checked_add(stats.mirror)
            .ok_or(ExecutorError::Overflow)?,
        baseline_live_paths: stats.baseline_live_paths,
        verdict: verdict(stats, input.limits.deletion)?,
    })
}

/// 复核整批后铸出放行票据；触发门槛即整批阻断。
pub(crate) fn clear(plan: &Plan) -> Result<(DeletionAudit, DeletionClearance), ExecutorError> {
    let audit = audit_batch(plan)?;
    match audit.verdict {
        Some(reason) => Err(ExecutorError::DeletionFuseBlocked(reason)),
        None => Ok((
            audit,
            DeletionClearance {
                deletions: audit.deletions,
            },
        )),
    }
}

/// 复用纯规划器的同一套门槛判定，避免执行侧出现第二个阈值实现。
pub(crate) fn verdict(
    stats: DeleteStats,
    limits: DeleteLimits,
) -> Result<Option<FuseReason>, ExecutorError> {
    crate::validation::fuse(stats, limits).map_err(|_| ExecutorError::Overflow)
}
