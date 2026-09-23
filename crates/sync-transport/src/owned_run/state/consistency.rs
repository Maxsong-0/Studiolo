//! 活动记录的阶段／证据自洽检查；`validate` 的唯一逐记录规则来源。

use super::{QuiescenceEvidence, RunRecord};
use crate::owned_run::error::{Result, TransportError};
use crate::owned_run::types::{ExitObservation, RunPhase};

pub(super) fn record_consistent(record: &RunRecord, next_sequence: u64) -> Result<()> {
    // 容器身份在 Contained 才固定；LaunchIssued 只知道请求，不知道真实身份。
    let container_required = matches!(
        record.phase,
        RunPhase::Contained
            | RunPhase::ExecutionIssued
            | RunPhase::Running
            | RunPhase::StopRequested
    );
    let launch_issued = record.phase.execution_may_have_started();
    let ok = record.sequence < next_sequence
        && launch_issued == record.requested_boot_id.is_some()
        && !record.intent.event_id.is_empty()
        && record.budget.complete()
        && (!container_required || record.container.is_some())
        && (record.container.is_none()
            || !matches!(record.phase, RunPhase::Prepared | RunPhase::Cancelled))
        // 已取消就不能带任何容器身份、退出结果或静止证明。
        && (record.phase != RunPhase::Cancelled
            || (record.exit == ExitObservation::NotObserved && record.quiescence.is_none()))
        // Quiesced 之后必须带真实静止证明，且入口已关闭。
        && (!matches!(
            record.phase,
            RunPhase::Quiesced | RunPhase::OutcomePending | RunPhase::Closed
        ) || record
            .quiescence
            .is_some_and(|proof| proof.spawn_gate_closed))
        // 子组空证明必须绑定实测子组身份；boot 变化不得冒充某个具体子组。
        && record.quiescence.is_none_or(|proof| match proof.evidence {
            QuiescenceEvidence::PayloadCgroupUnpopulated => {
                proof.cgroup_identity.is_some()
                    && proof.cgroup_identity
                        == record.container.as_ref().map(|c| c.cgroup_identity)
            }
            QuiescenceEvidence::BootChanged => true,
        })
        // 固定 Outcome 只在 Quiesced 之后出现；Closed 必须已 ACK。
        && (record.outcome.is_some()
            == matches!(
                record.phase,
                RunPhase::Cancelled | RunPhase::OutcomePending | RunPhase::Closed
            ))
        && (record.phase != RunPhase::Closed
            || record
                .outcome
                .as_ref()
                .is_some_and(|slot| !slot.pending()))
        // StopRequested 之后必须留有停止原因，取消优先于后来的 exit 0。
        && (record.phase != RunPhase::StopRequested || record.stop_reason.is_some());
    if ok {
        Ok(())
    } else {
        Err(TransportError::CorruptStore)
    }
}
