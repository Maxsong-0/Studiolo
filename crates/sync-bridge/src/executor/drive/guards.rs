//! 整批门禁：同一批、完整观察、冲突不被动作覆盖、删除必须先有静止。

use crate::executor::{
    deletion::DeletionAudit,
    error::ExecutorError,
    journal::JournalSnapshot,
    quiescence::{self, MirrorQuiescence, Quiesced, QuiescenceRequest},
};
use crate::{Change, Plan, Presence};
use std::time::Instant;

/// 核对冻结计划与持久快照确实是同一批；先于整批删除复核执行。
pub(super) fn alignment(plan: &Plan, snapshot: &JournalSnapshot) -> Result<(), ExecutorError> {
    if snapshot.fused {
        return Err(ExecutorError::Fused);
    }
    let sequence = plan.key().sequence;
    if sequence <= snapshot.retired_through {
        return Err(ExecutorError::Retired);
    }
    if snapshot.pending_sequence != Some(sequence) || snapshot.jobs.len() != plan.actions().len() {
        return Err(ExecutorError::PlanMismatch);
    }
    for (record, action) in snapshot.jobs.iter().zip(plan.actions()) {
        if record.key != action.key {
            return Err(ExecutorError::PlanMismatch);
        }
    }
    Ok(())
}

/// 整批删除复核之后才核对证据；冲突路径不得被任何动作覆盖。
pub(super) fn evidence(plan: &Plan) -> Result<(), ExecutorError> {
    observations(plan)?;
    conflicts(plan)
}

fn observations(plan: &Plan) -> Result<(), ExecutorError> {
    let input = plan.frozen_input();
    // 没有完整观察就不搬运，也不产生删除；缺失必须显式列出而不是漏项。
    if !input.canonical.complete
        || !input.mirror.complete
        || input.canonical.entries.len() != input.scope.paths.len()
        || input.mirror.entries.len() != input.scope.paths.len()
    {
        return Err(ExecutorError::IncompleteObservation);
    }
    for action in plan.actions() {
        let Change::Delete { .. } = action.change else {
            continue;
        };
        let path = &action.key.path;
        let source = match action.key.target {
            crate::Side::Canonical => input.mirror.entries.get(path),
            crate::Side::Mirror => input.canonical.entries.get(path),
        };
        // 删除只能来自「源侧显式 Missing ＋ 原共同基线存活」，不能来自漏项或未知。
        if !matches!(source, Some(Presence::Missing))
            || !input
                .baseline
                .entries
                .get(path)
                .is_some_and(|pair| matches!(pair.canonical, Presence::Present(_)))
        {
            return Err(ExecutorError::IncompleteObservation);
        }
    }
    Ok(())
}

fn conflicts(plan: &Plan) -> Result<(), ExecutorError> {
    for conflict in plan.conflicts() {
        if plan
            .actions()
            .iter()
            .any(|action| action.key.path == conflict.path)
        {
            return Err(ExecutorError::ConflictHasAction);
        }
    }
    Ok(())
}

/// 取静止证据；取不到时若本批含删除，错误必须点名「删除缺静止」。
pub(super) fn quiesce(
    plan: &Plan,
    port: &dyn MirrorQuiescence,
    audit: DeletionAudit,
    now: Instant,
    not_after: Instant,
) -> Result<Quiesced, ExecutorError> {
    let input = plan.frozen_input();
    let binding = &input.scope.binding;
    let request = QuiescenceRequest {
        link_id: binding.link_id,
        scope_epoch: binding.scope_epoch,
        mirror_root_token: binding.mirror_root_token,
        minimum_mirror_observation: input.mirror.sequence,
        not_after,
    };
    match port
        .observe(&request)
        .and_then(|proof| quiescence::admit(&request, &proof, now))
    {
        Ok(quiesced) => Ok(quiesced),
        Err(_) if audit.deletions > 0 => Err(ExecutorError::DeletionWithoutQuiescence),
        Err(fault) => Err(ExecutorError::QuiescenceMissing(fault)),
    }
}
