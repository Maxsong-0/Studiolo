//! 静止能力与已闭合运行的输出：只在 Outcome 已 ACK 之后才发，重开即作废。

use super::error::{Result, TransportError};
use super::session::RunSession;
use super::tickets::QuiescentMirror;
use super::types::{ExitObservation, Quiescence, RunId, RunOperation, RunPhase};
use crate::linux::ContainmentBackend;

impl<B: ContainmentBackend> RunSession<'_, B> {
    /// 已有静止证据且 Outcome 已 ACK 才发；再 prepare 立即作废。
    pub(crate) fn quiescent(&self) -> Result<QuiescentMirror> {
        let state = self.load()?;
        if state.active.is_some() {
            return Err(TransportError::AuditPending);
        }
        let terminal = state
            .last_terminal
            .as_ref()
            .ok_or(TransportError::NotFound)?;
        if terminal.observation.quiescence != Quiescence::Confirmed
            || terminal.observation.phase != RunPhase::Closed
        {
            return Err(TransportError::QuiescenceUnconfirmed);
        }
        // 静止票据交给 owner 的是视图路径：owner 从不直接触碰 payload 身份的真实根。
        let super::approval::RootPins {
            mirror_view_root,
            mirror_identity,
            ..
        } = state.roots.clone();
        Ok(QuiescentMirror {
            binding: self.ticket(&state, terminal.run_id, terminal.sequence)?,
            mirror_root: mirror_view_root,
            mirror_identity,
        })
    }

    /// 复验静止能力是否仍然有效；根变化、租约变化、owner 重开或新一轮 prepare 都会作废它。
    pub(crate) fn verify_quiescent(&self, mirror: &QuiescentMirror) -> Result<()> {
        let state = self.load()?;
        if mirror.binding.epoch != self.epoch || mirror.binding.lease != self.lease {
            return Err(TransportError::TicketForeign);
        }
        if mirror.binding.mirror_epoch != self.mirror_epoch
            || mirror.binding.config_digest != state.binding.approved_config_digest
            || mirror.binding.config_epoch != state.binding.scope_epoch
        {
            return Err(TransportError::TicketStale);
        }
        if mirror.binding.roots != state.roots
            || mirror.mirror_identity != state.roots.mirror_identity
        {
            return Err(TransportError::RootsChanged);
        }
        if state.active.is_some() {
            return Err(TransportError::TicketStale);
        }
        Ok(())
    }

    /// 已闭合运行的 stdout 正文；退出码非零、超预算或不是 InspectConfig／SetMode 一律不取。
    pub(crate) fn config_output(&mut self, run_id: RunId) -> Result<String> {
        let state = self.load()?;
        let terminal = state
            .last_terminal
            .as_ref()
            .filter(|terminal| terminal.run_id == run_id)
            .ok_or(TransportError::NotFound)?;
        if terminal.operation == RunOperation::RunOnce
            || terminal.observation.exit != ExitObservation::Code(0)
            || terminal.counts.output_over_budget
        {
            return Err(TransportError::Budget);
        }
        // B 轮 W1 实现：容器身份须来自持久记录。闭合摘要目前不保存容器身份，
        // W1 在 `TerminalSummary` 加 `container: Option<ContainerIdentity>` 后改为
        // `self.backend.config_output(&container).map_err(ContainmentGap::after_start)`。
        Err(TransportError::ExitUnconfirmed)
    }
}
