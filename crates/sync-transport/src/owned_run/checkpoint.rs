//! 原传输 owner 整体交出后封存；原槽/租约/运行时绑定始终保留，没有另开存储的捷径。
use super::{
    Result, RunAuditCandidate, SyncTransportOwner, TransportBindingFacts, TransportError,
    TransportState,
};
use super::{
    audit::{
        AuditPayload, AuditPhaseKind, OutcomeSummary, RequesterSource, binding_digest, event_id,
    },
    session::RunSession,
    state::encode,
};
use crate::linux::ContainmentBackend;
use std::{path::Path, time::Instant};

/// 原 owner 的冻结窗口；没有 prepare/start/recover 等可变入口。
pub struct FrozenTransport<'scope> {
    owner: SyncTransportOwner<'scope>,
    deadline: Instant,
    verified: Option<TransportState>,
}
impl<'scope> SyncTransportOwner<'scope> {
    /// 移入原 owner；失败由调用方经 thaw 返还原槽，不能因检查失败丢掉租约。
    #[must_use]
    pub fn freeze_checkpoint(self, deadline: Instant) -> FrozenTransport<'scope> {
        FrozenTransport {
            owner: self,
            deadline,
            verified: None,
        }
    }
}
impl<'scope> FrozenTransport<'scope> {
    /// 从实际固定状态证明没有活动运行，并返回原 link/scope 绑定供 Bridge 门禁核对。
    pub fn binding(&self) -> Result<TransportBindingFacts> {
        Ok(snapshot(&self.owner.inner, self.deadline)?.binding)
    }
    /// 核对最后闭合运行的两个完整原事件；原状态只保留闭合摘要，不能伪造不存在的 ACK。
    /// 回调须在对应 FrozenAudit 原链中查到逐字段一致的事件，不能只检查 event_id。
    pub fn verify_audit(
        &mut self,
        mut verify: impl FnMut(&RunAuditCandidate) -> Result<()>,
    ) -> Result<()> {
        self.verified = None;
        let before = snapshot(&self.owner.inner, self.deadline)?;
        for candidate in candidates(&self.owner.inner, &before)? {
            verify(&candidate)?;
        }
        if before != snapshot(&self.owner.inner, self.deadline)? {
            return Err(TransportError::CorruptStore);
        }
        self.verified = Some(before);
        Ok(())
    }
    /// 固定 state.json 独立导出；新 transport.lock 是空租约文件，原租约仍由本窗口持有。
    pub fn export(&self, target: &Path) -> Result<()> {
        let expected = self
            .verified
            .as_ref()
            .ok_or(TransportError::AuditRejected)?;
        if &snapshot(&self.owner.inner, self.deadline)? != expected {
            return Err(TransportError::CorruptStore);
        }
        self.owner
            .inner
            .publisher
            .export_checkpoint(target, &encode(expected)?, self.deadline)?;
        if &snapshot(&self.owner.inner, self.deadline)? != expected {
            return Err(TransportError::CorruptStore);
        }
        Ok(())
    }
    /// 返还原实例和原租约；没有重开、隐式恢复或源清理。
    #[must_use]
    pub fn thaw(self) -> SyncTransportOwner<'scope> {
        self.owner
    }
}
pub(crate) fn deadline(value: Instant) -> Result<()> {
    if Instant::now() >= value {
        Err(TransportError::Budget)
    } else {
        Ok(())
    }
}
pub(super) fn snapshot<B: ContainmentBackend>(
    session: &RunSession<'_, B>,
    until: Instant,
) -> Result<TransportState> {
    deadline(until)?;
    let state = session.load()?;
    if state.active.is_some() {
        return Err(TransportError::RunActive);
    }
    if state.last_terminal.is_some() {
        // 票据由本实例的真实持久 Closed/Confirmed 状态产生；原 owner 被移入后无法再 prepare。
        session.verify_quiescent(&session.quiescent()?)?;
    } else if state.generation != 1 || state.next_sequence != 1 {
        return Err(TransportError::QuiescenceUnconfirmed);
    }
    // 首次建库尚未分配过任何运行序号，同一原租约下没有已发出的 spawn。
    deadline(until)?;
    Ok(state)
}
pub(super) fn candidates<B: ContainmentBackend>(
    session: &RunSession<'_, B>,
    state: &TransportState,
) -> Result<Vec<RunAuditCandidate>> {
    let Some(terminal) = &state.last_terminal else {
        return Ok(Vec::new());
    };
    [AuditPhaseKind::Intent, AuditPhaseKind::Outcome]
        .into_iter()
        .map(|phase| {
            Ok(RunAuditCandidate {
                binding: session.ticket(state, terminal.run_id, terminal.sequence)?,
                phase,
                event_id: event_id(terminal.run_id, phase),
                payload: AuditPayload {
                    schema_version: 1,
                    run_id: terminal.run_id,
                    sequence: terminal.sequence,
                    operation: terminal.operation,
                    binding_digest: binding_digest(&state.binding),
                    runtime_digest: session.runtime.digest(),
                    backend_digest: state.backend.digest,
                    budget: session.approval.budget(),
                    requester: RequesterSource::OwnerSession,
                    outcome: (phase == AuditPhaseKind::Outcome).then_some(OutcomeSummary {
                        container_created: terminal.container.is_some(),
                        stop_reason: terminal.observation.stop_reason,
                        exit_observed: terminal.observation.exit.reaped(),
                        quiescence: terminal.observation.quiescence,
                        counts: terminal.counts,
                    }),
                },
            })
        })
        .collect()
}
