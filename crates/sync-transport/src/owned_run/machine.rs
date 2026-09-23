//! 纯状态迁移：每一次转换都产出一份完整的新记录，由调用方原子发布后才生效。
//!
//! 这里没有任何 IO。阶段边界严格照契约：`LaunchIssued` 在请求 launcher 之前，
//! `ExecutionIssued` 在放行屏障之前；静止只认「入口已关闭 + 同一子组实测为空」。

use super::audit::{AuditPhaseKind, event_id, outcome_summary};
use super::error::{Result, TransportError};
use super::state::{
    AuditReceipt, AuditSlot, QuiescenceEvidence, QuiescenceProof, RunRecord, TransportState,
    validate,
};
use super::types::{
    ContainerIdentity, ExitObservation, ResourceCounts, RunDiagnostics, RunId, RunOperation,
    RunPhase, StopReason,
};

/// 一次准备请求。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrepareRequest {
    /// 新分配的随机运行标识。
    pub run_id: RunId,
    /// 固定操作。
    pub operation: RunOperation,
    /// 调用方提供的请求时刻毫秒数；本 crate 不读时钟。
    pub requested_at_millis: u64,
    /// 调用方期望的下一个序号。
    pub expected_sequence: u64,
    /// 本次运行预算；缺项不得启动。
    pub budget: super::approval::RunBudget,
}

/// 所有被允许的转换；没有「回到 Running」或「清空停止原因」的分支。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Transition {
    /// 尚未 LaunchIssued 的持久取消，固定「未执行」Outcome。
    Cancel,
    /// 在请求启动 launcher **之前**持久；同时固定当时的 boot 标识。
    IssueLaunch {
        /// 请求启动时的 boot 标识。
        boot_id: [u8; 16],
    },
    /// 屏障内可信 launcher 已建子组，owner 固定身份并核对隔离/预算。
    Contain {
        /// 实测容器身份。
        container: Box<ContainerIdentity>,
        /// 仅诊断字段。
        diagnostics: RunDiagnostics,
    },
    /// 在放行屏障**之前**持久；协议只消费一次放行。
    IssueExecution,
    /// 来自实际执行观察。
    ObserveRunning,
    /// 停止原因先持久；第一个被持久的原因就是最终原因。
    RequestStop(StopReason),
    /// 独立保存的真实等待结果。
    RecordExit(ExitObservation),
    /// 实测资源计数；只增不减。
    RecordCounts(ResourceCounts),
    /// 入口已关闭且同一子组实测为空。
    Quiesce(QuiescenceProof),
    /// 固定 Outcome 可靠保存。
    FixOutcome,
    /// 已核验的真实审计回执。
    Acknowledge {
        /// 事件阶段。
        phase: AuditPhaseKind,
        /// 已核验回执。
        receipt: AuditReceipt,
    },
}

/// 准备一次运行；state/Intent 已可靠发布，但没有任何启动命令。
///
/// # 错误
/// 活动槽未空返回 `RunActive`；序号不符返回 `SequenceMismatch`；计数溢出返回 `CounterOverflow`。
pub fn prepare(state: &TransportState, request: &PrepareRequest) -> Result<TransportState> {
    if state.active.is_some() {
        return Err(TransportError::RunActive);
    }
    if request.expected_sequence != state.next_sequence {
        return Err(TransportError::SequenceMismatch);
    }
    if !request.budget.complete() {
        return Err(TransportError::Budget);
    }
    let mut next = state.clone();
    next.generation = bump(state.generation)?;
    next.next_sequence = bump(state.next_sequence)?;
    next.active = Some(RunRecord {
        run_id: request.run_id,
        sequence: request.expected_sequence,
        operation: request.operation,
        requested_at_millis: request.requested_at_millis,
        budget: request.budget,
        phase: RunPhase::Prepared,
        stop_reason: None,
        requested_boot_id: None,
        container: None,
        diagnostics: RunDiagnostics::default(),
        exit: ExitObservation::NotObserved,
        quiescence: None,
        counts: ResourceCounts::default(),
        intent: AuditSlot {
            event_id: event_id(request.run_id, AuditPhaseKind::Intent),
            receipt: None,
        },
        outcome: None,
    });
    validate(&next)?;
    Ok(next)
}

/// 应用一次转换；产出完整新记录，调用方负责原子发布。
///
/// # 错误
/// 阶段不允许返回 `PhaseConflict`；Intent 未 ACK 就启动返回 `AuditPending`。
pub fn apply(state: &TransportState, transition: Transition) -> Result<TransportState> {
    let mut next = state.clone();
    next.generation = bump(state.generation)?;
    let record = next.active.as_mut().ok_or(TransportError::NotFound)?;
    let retire = step(record, transition)?;
    if retire {
        let terminal = record.terminal();
        next.last_terminal = Some(terminal);
        next.active = None;
    }
    validate(&next)?;
    Ok(next)
}

fn step(record: &mut RunRecord, transition: Transition) -> Result<bool> {
    match transition {
        Transition::Cancel => {
            if record.phase != RunPhase::Prepared {
                return Err(TransportError::PhaseConflict);
            }
            record.phase = RunPhase::Cancelled;
            record.outcome = Some(outcome_slot(record.run_id));
        }
        Transition::IssueLaunch { boot_id } => {
            if record.phase != RunPhase::Prepared {
                return Err(TransportError::PhaseConflict);
            }
            // Intent 没有 ACK 不启动：这一条挡在任何启动请求之前。
            if record.intent.pending() {
                return Err(TransportError::AuditPending);
            }
            record.requested_boot_id = Some(boot_id);
            record.phase = RunPhase::LaunchIssued;
        }
        Transition::Contain {
            container,
            diagnostics,
        } => {
            if record.phase != RunPhase::LaunchIssued {
                return Err(TransportError::PhaseConflict);
            }
            record.container = Some(*container);
            record.diagnostics = diagnostics;
            record.phase = RunPhase::Contained;
        }
        Transition::IssueExecution => {
            if record.phase != RunPhase::Contained {
                return Err(TransportError::PhaseConflict);
            }
            record.phase = RunPhase::ExecutionIssued;
        }
        Transition::ObserveRunning => {
            if record.phase != RunPhase::ExecutionIssued {
                return Err(TransportError::PhaseConflict);
            }
            record.phase = RunPhase::Running;
        }
        Transition::RequestStop(reason) => {
            if !matches!(
                record.phase,
                RunPhase::LaunchIssued
                    | RunPhase::Contained
                    | RunPhase::ExecutionIssued
                    | RunPhase::Running
                    | RunPhase::StopRequested
            ) {
                return Err(TransportError::PhaseConflict);
            }
            // 第一个被持久的原因就是最终原因；后来的请求不覆盖，也不清空。
            record.stop_reason.get_or_insert(reason);
            record.phase = RunPhase::StopRequested;
        }
        Transition::RecordExit(exit) => {
            if !record.phase.release_consumed() {
                return Err(TransportError::PhaseConflict);
            }
            match (record.exit, exit) {
                // 已取得的真实等待结果不得被「未观察到」覆盖。
                (_, ExitObservation::NotObserved) => {}
                (ExitObservation::NotObserved, observed) => record.exit = observed,
                (old, new) if old == new => {}
                _ => return Err(TransportError::PhaseConflict),
            }
        }
        Transition::RecordCounts(counts) => {
            if !record.phase.release_consumed() {
                return Err(TransportError::PhaseConflict);
            }
            if counts.stdout_bytes < record.counts.stdout_bytes
                || counts.stderr_bytes < record.counts.stderr_bytes
            {
                return Err(TransportError::PhaseConflict);
            }
            record.counts = ResourceCounts {
                output_over_budget: record.counts.output_over_budget || counts.output_over_budget,
                ..counts
            };
        }
        Transition::Quiesce(proof) => quiesce(record, proof)?,
        Transition::FixOutcome => {
            if record.phase != RunPhase::Quiesced {
                return Err(TransportError::PhaseConflict);
            }
            record.outcome = Some(outcome_slot(record.run_id));
            record.phase = RunPhase::OutcomePending;
        }
        Transition::Acknowledge { phase, receipt } => return acknowledge(record, phase, receipt),
    }
    Ok(false)
}

fn quiesce(record: &mut RunRecord, proof: QuiescenceProof) -> Result<()> {
    if !matches!(
        record.phase,
        RunPhase::LaunchIssued
            | RunPhase::Contained
            | RunPhase::ExecutionIssued
            | RunPhase::Running
            | RunPhase::StopRequested
    ) {
        return Err(TransportError::PhaseConflict);
    }
    // 没有关闭继续 spawn 的入口就不是静止证明。
    if !proof.spawn_gate_closed {
        return Err(TransportError::QuiescenceUnconfirmed);
    }
    if proof.evidence == QuiescenceEvidence::PayloadCgroupUnpopulated
        && proof.cgroup_identity != record.container.as_ref().map(|c| c.cgroup_identity)
    {
        return Err(TransportError::ExitUnconfirmed);
    }
    record.quiescence = Some(proof);
    record.phase = RunPhase::Quiesced;
    Ok(())
}

fn acknowledge(
    record: &mut RunRecord,
    phase: AuditPhaseKind,
    receipt: AuditReceipt,
) -> Result<bool> {
    match phase {
        AuditPhaseKind::Intent => {
            if !record.intent.pending() {
                return Err(TransportError::AuditMismatch);
            }
            record.intent.receipt = Some(receipt);
            Ok(false)
        }
        AuditPhaseKind::Outcome => {
            if !matches!(record.phase, RunPhase::Cancelled | RunPhase::OutcomePending) {
                return Err(TransportError::PhaseConflict);
            }
            // Intent 必须先闭合；两个待补事件不得乱序补齐。
            if record.intent.pending() {
                return Err(TransportError::AuditPending);
            }
            let slot = record.outcome.as_mut().ok_or(TransportError::NotFound)?;
            if !slot.pending() {
                return Err(TransportError::AuditMismatch);
            }
            slot.receipt = Some(receipt);
            record.phase = RunPhase::Closed;
            Ok(true)
        }
    }
}

/// 当前待补审计事件的阶段；一个活动槽最多两个。
#[must_use]
pub fn pending_phase(record: &RunRecord) -> Option<AuditPhaseKind> {
    if record.intent.pending() {
        return Some(AuditPhaseKind::Intent);
    }
    if record.outcome.as_ref().is_some_and(AuditSlot::pending) {
        return Some(AuditPhaseKind::Outcome);
    }
    None
}

/// 闭合摘要在审计确认前后都必须一致；用于比对提交的候选。
#[must_use]
pub fn outcome_facts(record: &RunRecord) -> super::audit::OutcomeSummary {
    outcome_summary(record)
}

fn outcome_slot(run_id: RunId) -> AuditSlot {
    AuditSlot {
        event_id: event_id(run_id, AuditPhaseKind::Outcome),
        receipt: None,
    }
}

fn bump(value: u64) -> Result<u64> {
    value.checked_add(1).ok_or(TransportError::CounterOverflow)
}
