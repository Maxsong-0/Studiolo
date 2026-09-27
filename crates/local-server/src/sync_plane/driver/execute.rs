//! 第 10～12 步：`BridgeState::reserve` → `StateJournal` → `executor::execute` → 保留期回收。
//!
//! 上一批未闭合（重开续做）就沿用冻结原输入，不重新预留；执行器自己保证 Issued 不重放。

use super::super::PlaneError;
use super::super::job_backend::{PlaneJobBackend, SharedStageSink};
use super::super::mirror::PlaneContent;
use super::super::quiescence::OwnerQuiescence;
use super::super::witness::{PlaneWitness, WitnessContext};
use super::RoundOutcome;
use super::content::RoundContext;
use super::observe::Observed;
use std::time::Instant;
use studiolo_sync_bridge::executor::{
    CasLimits, ExecutionPorts, ExecutorError, JournalFault, MirrorCas, execute,
};
use studiolo_sync_bridge::sqlite::{BridgeError, Reservation, StateJournal};
use studiolo_sync_bridge::{Decision, plan};

/// 预留并执行一批；返回本轮结论。
///
/// # Errors
/// 熔断返回 `Fused`；有未知终态不返回错误而是 `Unknown` 结论；其余按端口失败分类。
pub fn reserve_and_execute(
    context: &RoundContext<'_>,
    observed: &Observed,
) -> Result<RoundOutcome, PlaneError> {
    let plane = context.plane;
    // 11 先把 canonical 写者的两条审计 outbox 推到真实 ACK；推不动就不发任何新动作。
    plane
        .workbench()
        .bridge_flush_audit()
        .map_err(|_| PlaneError::Audit)?;
    let mut bridge = plane.bridge().lock().map_err(|_| PlaneError::Unavailable)?;
    let view_state = bridge.view().map_err(bridge_error)?;
    let resumed = view_state.pending_sequence.is_some();
    let input = if resumed {
        bridge
            .frozen_pending()
            .map_err(bridge_error)?
            .ok_or(PlaneError::Storage)?
    } else {
        observed.input.clone()
    };
    let decision = plan(input.clone(), None).map_err(|_| PlaneError::Observation)?;
    let plan = match decision {
        Decision::Ready(plan) => plan,
        Decision::FuseBlocked(_) => {
            // 熔断先原子持久，再报 Fused；本片没有解除入口。
            if !resumed {
                bridge
                    .reserve(view_state.version, input)
                    .map_err(bridge_error)?;
            }
            return Err(PlaneError::Fused);
        }
    };
    if !resumed {
        if plan.actions().is_empty()
            && plan.conflicts().is_empty()
            && input.baseline.entries.len() == input.scope.paths.len()
        {
            return Ok(RoundOutcome::Idle);
        }
        if let Reservation::Fused { .. } = bridge
            .reserve(view_state.version, input.clone())
            .map_err(bridge_error)?
        {
            return Err(PlaneError::Fused);
        }
    }
    let sequence = plan.key().sequence;
    let binding = input.scope.binding.clone();
    let quiescence = OwnerQuiescence::take(
        plane,
        binding.link_id,
        binding.scope_epoch,
        binding.mirror_root_token,
        observed.input.mirror.sequence.max(input.mirror.sequence),
        Instant::now(),
    )?;
    let mut witness = PlaneWitness::new(
        plane,
        context.view.clone(),
        context.ticket,
        WitnessContext {
            binding: binding.clone(),
            approved: input.scope.clone(),
            baseline: input.baseline.clone(),
            epoch: observed.epoch,
            next_canonical: observed.next_canonical,
            next_mirror: observed.next_mirror,
        },
    );
    let outcome = {
        let mut journal = StateJournal::new(&mut bridge, plane.audit().as_ref(), input.clone());
        let mut content = PlaneContent::new(plane, &context.view);
        let mut sink = SharedStageSink::new(context.stage);
        let mut backend = PlaneJobBackend::new(context, binding);
        let mut cas = MirrorCas::new(CasLimits::internal());
        let ports = ExecutionPorts {
            quiescence: &quiescence,
            journal: &mut journal,
            witness: &mut witness,
            content: &mut content,
            sink: &mut sink,
            backend: &mut backend,
            cas: &mut cas,
        };
        let report = execute(&plan, ports, Instant::now(), quiescence.expires_at())
            .map_err(executor_error)?;
        if report.unknown > 0 {
            RoundOutcome::Unknown { sequence }
        } else if report.settled {
            RoundOutcome::Settled {
                sequence,
                baseline_advanced: report.baseline_advanced,
            }
        } else {
            RoundOutcome::Blocked { sequence }
        }
    };
    let retired_through = bridge.view().map(|view| view.retired_through).unwrap_or(0);
    drop(bridge);
    // 12 保留期回收：IO 失败不改变本批结论，下一轮再扫。
    let _ = context
        .stage
        .borrow_mut()
        .sweep(context.now_unix, retired_through);
    Ok(outcome)
}

/// 持久层失败到平面失败：熔断、审计各归其类，预算／绑定不符是装配问题，其余是存储。
pub fn bridge_error(error: BridgeError) -> PlaneError {
    match error {
        BridgeError::Fused => PlaneError::Fused,
        BridgeError::AuditUnavailable => PlaneError::Audit,
        BridgeError::VersionConflict | BridgeError::BindingMismatch => PlaneError::Configuration,
        BridgeError::StaleObservation
        | BridgeError::PendingUnresolved
        | BridgeError::RetryMismatch => PlaneError::Observation,
        _ => PlaneError::Storage,
    }
}

/// 执行器失败到平面失败；未知终态不是失败，不会走到这里。
pub fn executor_error(error: ExecutorError) -> PlaneError {
    match error {
        ExecutorError::Fused
        | ExecutorError::DeletionFuseBlocked(_)
        | ExecutorError::Journal(JournalFault::Fused) => PlaneError::Fused,
        ExecutorError::QuiescenceMissing(_) | ExecutorError::DeletionWithoutQuiescence => {
            PlaneError::Quiescence
        }
        ExecutorError::IncompleteObservation
        | ExecutorError::Witness(_)
        | ExecutorError::Journal(JournalFault::StaleObservation) => PlaneError::Observation,
        ExecutorError::AuditStalled
        | ExecutorError::Journal(JournalFault::AuditUnavailable | JournalFault::IntentRequired) => {
            PlaneError::Audit
        }
        ExecutorError::Transfer(_) | ExecutorError::Cas(_) => PlaneError::Unavailable,
        ExecutorError::Retired
        | ExecutorError::PlanMismatch
        | ExecutorError::ConflictHasAction
        | ExecutorError::Overflow
        | ExecutorError::Journal(_) => PlaneError::Storage,
    }
}
